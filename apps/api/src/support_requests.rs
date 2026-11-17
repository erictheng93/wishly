//! 客服工單（#92；與「支持品項」的 support.rs 無關）。會員走 `/me/support-requests`，未登入（含被停權而無法登入者）
//! 走 `/public/support-requests`（不讀 cookie、Turnstile、只收 Email）。營運單次回覆即結案（`/admin/support-requests`）。
//! 會員工單結案發 `support.replied`（站內）；匿名工單的 Email 未經驗證，系統不寄信，營運用客服信箱自行回覆。
use crate::{
    admin::{audit, page, Staff, SUPPORT},
    error::AppError,
    ratelimit,
    session::CurrentUser,
    AppState,
};
use axum::{
    extract::{Path, Query, State},
    http::{request::Parts, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, patch, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/me/support-requests", get(mine).post(create))
        .route("/public/support-requests", post(create_public))
        .route("/admin/support-requests", get(list))
        .route("/admin/support-requests/{id}", patch(reply))
}

const CATEGORIES: [&str; 5] = ["account", "wishlist", "payment", "wallet", "other"];
const MAX_OPEN_USER: i64 = 5;
const MAX_OPEN_EMAIL: i64 = 3;

fn validate(category: &str, body: &str) -> Result<String, AppError> {
    if !CATEGORIES.contains(&category) {
        return Err(AppError::invalid("/category", "INVALID", "category 不合法"));
    }
    let body = crate::validate::text(body, "/body", true)?;
    if body.is_empty() {
        return Err(AppError::invalid("/body", "REQUIRED", "請填寫內容"));
    }
    if body.chars().count() > 2000 {
        return Err(AppError::invalid("/body", "TOO_LONG", "最多 2000 字"));
    }
    Ok(body)
}

fn open_limit() -> AppError {
    AppError::problem(
        409,
        "SUPPORT_OPEN_LIMIT",
        "處理中的客服請求已達上限，請等候回覆後再送出",
    )
}

// 使用者看得到的欄位：不含 handled_by（不洩漏營運身分）
const MINE: &str = "jsonb_build_object('id', id, 'category', category, 'body', body, 'wishlist_id', wishlist_id, 'topup_id', topup_id,
    'status', status, 'reply', reply, 'handled_at', handled_at, 'created_at', created_at)";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewRequest {
    category: String,
    body: String,
    wishlist_id: Option<Uuid>,
    topup_id: Option<Uuid>,
}

async fn create(
    State(st): State<AppState>,
    u: CurrentUser,
    Json(b): Json<NewRequest>,
) -> Result<Response, AppError> {
    let body = validate(&b.category, &b.body)?;
    // 參照必須是自己能管理的清單 / 自己的儲值單；不存在與無權限同回 422，避免探測別人的資源
    let (wl_ok, tp_ok): (bool, bool) = sqlx::query_as(
        "SELECT $1::uuid IS NULL OR EXISTS (SELECT 1 FROM wishlists w WHERE w.id = $1 AND can_manage_wishlist(w.owner_id, w.org_id, $3)),
                $2::uuid IS NULL OR EXISTS (SELECT 1 FROM topups t JOIN point_wallets pw ON pw.id = t.wallet_id WHERE t.id = $2 AND pw.user_id = $3)")
        .bind(b.wishlist_id).bind(b.topup_id).bind(u.id).fetch_one(&st.pool).await?;
    if !wl_ok {
        return Err(AppError::invalid(
            "/wishlist_id",
            "INVALID",
            "找不到這份清單",
        ));
    }
    if !tp_ok {
        return Err(AppError::invalid("/topup_id", "INVALID", "找不到這筆儲值"));
    }
    ratelimit::check_pool(&st.pool, &format!("support_user:{}", u.id), 10, 3600).await?;
    // shortcut: 計數與 INSERT 同一句但非鎖定，同一人併發送出可能略超過上限（每小時 10 筆的限流兜底）；這只是防灌單，不需精確
    let row: Option<Value> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "INSERT INTO support_requests (user_id, category, body, wishlist_id, topup_id)
         SELECT $1, $2, $3, $4, $5
          WHERE (SELECT count(*) FROM support_requests WHERE user_id = $1 AND status = 'open') < $6
         RETURNING {MINE}"
    )))
    .bind(u.id)
    .bind(&b.category)
    .bind(&body)
    .bind(b.wishlist_id)
    .bind(b.topup_id)
    .bind(MAX_OPEN_USER)
    .fetch_optional(&st.pool)
    .await?;
    let v = row.ok_or_else(open_limit)?;
    Ok((StatusCode::CREATED, Json(v)).into_response())
}

async fn mine(State(st): State<AppState>, u: CurrentUser) -> Result<Json<Value>, AppError> {
    let rows: Vec<Value> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT {MINE} FROM support_requests WHERE user_id = $1 ORDER BY created_at DESC LIMIT 50"
    )))
    .bind(u.id)
    .fetch_all(&st.pool)
    .await?;
    Ok(Json(json!({ "data": rows })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewPublicRequest {
    email: String,
    category: String,
    body: String,
    turnstile_token: Option<String>,
}

/// 未登入表單：不讀 cookie、不查 users（回應不因 Email 是否已註冊而不同，也不掛到該帳號：Email 未經驗證）。
async fn create_public(
    State(st): State<AppState>,
    parts: Parts,
    Json(b): Json<NewPublicRequest>,
) -> Result<Response, AppError> {
    let email = crate::auth::norm_email(&b.email)?;
    let body = validate(&b.category, &b.body)?;
    let ip = crate::config::client_ip(&parts, crate::config::get());
    ratelimit::check_pool(&st.pool, &format!("support_ip:{ip}"), 30, 3600).await?;
    crate::auth::require_captcha(b.turnstile_token.as_deref(), &ip).await?;
    // shortcut: 同上，併發可能略超過上限，由每 IP 限流兜底
    let id: Option<Uuid> = sqlx::query_scalar(
        "INSERT INTO support_requests (contact_email, category, body)
         SELECT $1, $2, $3
          WHERE (SELECT count(*) FROM support_requests WHERE contact_email = $1 AND status = 'open') < $4
         RETURNING id",
    )
    .bind(&email)
    .bind(&b.category)
    .bind(&body)
    .bind(MAX_OPEN_EMAIL)
    .fetch_optional(&st.pool)
    .await?;
    let id = id.ok_or_else(open_limit)?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "id": id, "status": "open" })),
    )
        .into_response())
}

// 後台：會員工單帶使用者 id / 名稱 / handle（不帶 Email）；匿名工單只帶 contact_email（唯一聯絡方式）
const ADMIN_SQL: &str = "SELECT r.id, jsonb_build_object('id', r.id, 'kind', CASE WHEN r.user_id IS NULL THEN 'guest' ELSE 'member' END,
    'user', CASE WHEN u.id IS NULL THEN NULL ELSE jsonb_build_object('id', u.id, 'handle', u.handle, 'display_name', u.display_name) END,
    'contact_email', r.contact_email, 'category', r.category, 'body', r.body, 'wishlist_id', r.wishlist_id, 'topup_id', r.topup_id,
    'status', r.status, 'reply', r.reply, 'handled_at', r.handled_at,
    'handled_by', CASE WHEN h.id IS NULL THEN NULL ELSE jsonb_build_object('id', h.id, 'display_name', h.display_name) END,
    'created_at', r.created_at)
  FROM support_requests r LEFT JOIN users u ON u.id = r.user_id LEFT JOIN users h ON h.id = r.handled_by";

#[derive(Deserialize)]
struct ListQ {
    status: Option<String>,
    cursor: Option<Uuid>,
    limit: Option<i64>,
}

async fn list(
    State(st): State<AppState>,
    _s: Staff,
    Query(q): Query<ListQ>,
) -> Result<Response, AppError> {
    // open 依先來後到處理；closed 看最近處理的（id 為 uuid v7，即建立順序）
    let (status, cmp, dir) = match q.status.as_deref().unwrap_or("open") {
        "open" => ("open", ">", "ASC"),
        "closed" => ("closed", "<", "DESC"),
        _ => return Err(AppError::invalid("/status", "INVALID", "open | closed")),
    };
    let lim = q.limit.unwrap_or(20).clamp(1, 100);
    let mut tx = crate::staff_db::begin_read(&st.pool).await?;
    let rows: Vec<(Uuid, Value)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "{ADMIN_SQL} WHERE r.status = $1 AND ($2::uuid IS NULL OR r.id {cmp} $2) ORDER BY r.id {dir} LIMIT $3"
    )))
    .bind(status)
    .bind(q.cursor)
    .bind(lim + 1)
    .fetch_all(&mut *tx)
    .await?;
    Ok(page(rows, lim))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Reply {
    reply: String,
}

async fn reply(
    State(st): State<AppState>,
    Staff(sid): Staff<SUPPORT>,
    Path(id): Path<Uuid>,
    Json(b): Json<Reply>,
) -> Result<Json<Value>, AppError> {
    let text = crate::validate::optional_note(Some(&b.reply), "/reply", 2000)?
        .ok_or_else(|| AppError::invalid("/reply", "REQUIRED", "請填寫回覆"))?;
    let mut tx = st.pool.begin().await?;
    // 條件式 UPDATE：並行回覆只有一個成功，另一個看到已結案回 409
    let done: Option<Option<Uuid>> = sqlx::query_scalar(
        "UPDATE support_requests SET status = 'closed', reply = $2, handled_by = $3, handled_at = now()
          WHERE id = $1 AND status = 'open' RETURNING user_id",
    )
    .bind(id)
    .bind(&text)
    .bind(sid)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(user_id) = done else {
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM support_requests WHERE id = $1)")
                .bind(id)
                .fetch_one(&mut *tx)
                .await?;
        return Err(if exists {
            AppError::problem(409, "INVALID_STATE_TRANSITION", "客服請求已結案")
        } else {
            AppError::NotFound
        });
    };
    // 匿名工單（user_id 為空）不通知：Email 未驗證，系統不對其寄信
    if let Some(uid) = user_id {
        crate::notify::enqueue(&mut *tx, uid, "support.replied", json!({})).await?;
    }
    // diff 只記狀態變化，不放內文、回覆與 Email
    audit(
        &mut *tx,
        sid,
        "support_request.reply",
        "support_requests",
        Some(id),
        json!({ "status": { "before": "open", "after": "closed" } }),
    )
    .await?;
    let (_, v): (Uuid, Value) =
        sqlx::query_as(sqlx::AssertSqlSafe(format!("{ADMIN_SQL} WHERE r.id = $1")))
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
    tx.commit().await?;
    Ok(Json(v))
}
