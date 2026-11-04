//! 檢舉（公開）+ staff 處理檢舉 / 下架與恢復清單（docs/04 4.11）。
use crate::{
    admin::{audit, page, Staff},
    dashboard,
    error::AppError,
    ratelimit,
    session::{cookie_value, hash_token, CurrentUser},
    AppState,
};
use axum::{
    extract::{FromRequestParts, Path, Query, State},
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
        .route("/public/wishlists/{slug}/reports", post(create))
        .route("/public/orgs/{slug}/reports", post(create_org))
        .route("/admin/reports", get(list))
        .route("/admin/reports/targets", get(targets))
        .route(
            "/admin/users/{id}/moderation-history",
            get(moderation_history),
        )
        .route("/admin/reports/{id}", patch(handle))
        .route("/admin/wishlists/{id}/moderation", patch(moderate))
}

#[derive(Deserialize)]
struct NewReport {
    reason: String,
    detail: Option<String>,
    item_id: Option<Uuid>,
    turnstile_token: Option<String>,
}

/// Turnstile siteverify。未設 secret（僅 dev/test）略過。契約錯誤碼表：Turnstile 失敗 = 403 FORBIDDEN（不是 422，
/// 因為欄位格式本身合法、是人機驗證不通過）；siteverify 連不上 = 503 CAPTCHA_UNAVAILABLE（fail closed，不放行；不用通用的 SERVICE_UNAVAILABLE，前端才能說清楚是哪個服務）。secret 不寫入 log。
pub async fn verify_turnstile(
    cfg: &crate::config::Config,
    token: Option<&str>,
    ip: &str,
) -> Result<(), AppError> {
    let Some(secret) = cfg.turnstile_secret.as_deref() else {
        return Ok(());
    };
    let fail = || AppError::problem(403, "FORBIDDEN", "人機驗證失敗，請重新整理後再試");
    let token = token
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .ok_or_else(fail)?;
    let mut form = vec![("secret", secret), ("response", token)];
    if ip != "unknown" {
        form.push(("remoteip", ip));
    }
    let res = reqwest::Client::new()
        .post(&cfg.turnstile_verify_url)
        .form(&form)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await;
    let v: Value = match res {
        Ok(r) if r.status().is_success() => r.json().await.unwrap_or(Value::Null),
        Ok(r) => {
            tracing::error!(status = %r.status(), "turnstile siteverify");
            return Err(AppError::problem(
                503,
                "CAPTCHA_UNAVAILABLE",
                "人機驗證服務暫時無法使用",
            ));
        }
        Err(e) => {
            tracing::error!(error = %e.without_url(), "turnstile siteverify");
            return Err(AppError::problem(
                503,
                "CAPTCHA_UNAVAILABLE",
                "人機驗證服務暫時無法使用",
            ));
        }
    };
    if v["success"] == true {
        Ok(())
    } else {
        Err(fail())
    }
}

/// reason 與 detail 驗證（清單 / 組織檢舉共用）；回傳整理後的 detail。
fn validate_body(reason: &str, detail: Option<&str>) -> Result<Option<String>, AppError> {
    if !REASONS.contains(&reason) {
        return Err(AppError::invalid("/reason", "INVALID", "reason 不合法"));
    }
    let detail = match detail {
        Some(d) => Some(crate::validate::text(d, "/detail", true)?).filter(|d| !d.is_empty()),
        None => None,
    };
    if detail.as_deref().is_some_and(|d| d.chars().count() > 1000) {
        return Err(AppError::invalid("/detail", "TOO_LONG", "最多 1000 字"));
    }
    Ok(detail)
}

const REASONS: [&str; 5] = [
    "scam",
    "inappropriate",
    "copyright",
    "personal_info",
    "other",
];

/// 檢舉者（選填）：session 或 guest token（X-Guest-Token / ws_guest，DB 存 SHA-256(token 字串)）
async fn reporter(
    parts: &mut Parts,
    st: &AppState,
) -> Result<(Option<Uuid>, Option<Uuid>), AppError> {
    let user = CurrentUser::from_request_parts(parts, st)
        .await
        .ok()
        .map(|u| u.id);
    let mut guest: Option<Uuid> = None;
    if user.is_none() {
        let tok = parts
            .headers
            .get("x-guest-token")
            .and_then(|v| v.to_str().ok())
            .or_else(|| cookie_value(parts, "ws_guest"));
        if let Some(t) = tok {
            guest = sqlx::query_scalar(
                "SELECT id FROM guests WHERE guest_token_hash = $1 AND deleted_at IS NULL",
            )
            .bind(hash_token(t))
            .fetch_optional(&st.pool)
            .await?;
        }
    }
    Ok((user, guest))
}

/// 公開檢舉公益組織（僅 verified；路由受 ORGS_ENABLED 閘門，見 lib.rs::orgs_gate）。流程同 `create`，目標是組織。
async fn create_org(
    State(st): State<AppState>,
    Path(slug): Path<String>,
    mut parts: Parts,
    Json(b): Json<NewReport>,
) -> Result<Response, AppError> {
    let detail = validate_body(&b.reason, b.detail.as_deref())?;
    if !crate::validate::slug_ok(&slug) {
        return Err(AppError::NotFound);
    }
    let oid: Uuid = sqlx::query_scalar(
        "SELECT id FROM organizations WHERE slug = $1 AND verification_status = 'verified'",
    )
    .bind(&slug)
    .fetch_optional(&st.pool)
    .await?
    .ok_or(AppError::NotFound)?;
    let (user, guest) = reporter(&mut parts, &st).await?;
    if user.is_some() || guest.is_some() {
        let dup: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM content_reports WHERE reported_org_id = $1 AND created_at > now() - interval '24 hours'
               AND reporter_user_id IS NOT DISTINCT FROM $2 AND reporter_guest_id IS NOT DISTINCT FROM $3 LIMIT 1")
            .bind(oid).bind(user).bind(guest).fetch_optional(&st.pool).await?;
        if let Some(id) = dup {
            return Ok((
                StatusCode::CREATED,
                Json(json!({ "id": id, "status": "open" })),
            )
                .into_response());
        }
    }
    let ip = crate::config::client_ip(&parts, &crate::config::get());
    ratelimit::check(&st.pool, &format!("report_ip:{ip}"), 10, 3600).await?;
    verify_turnstile(&crate::config::get(), b.turnstile_token.as_deref(), &ip).await?;
    if user.is_none() && guest.is_none() {
        ratelimit::check(&st.pool, &format!("report_org:{oid}:{ip}"), 1, 86400).await?;
    }
    // 快照只存 logo key（輸出才換 URL）；不含聯絡 email / 電話
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO content_reports (reported_org_id, reason, detail, reporter_user_id, reporter_guest_id, snapshot)
         SELECT id, $2::report_reason, $3, $4, $5,
           jsonb_build_object('display_name', display_name, 'slug', slug::text, 'intro', intro, 'website', website, 'logo_key', logo_key)
         FROM organizations WHERE id = $1 RETURNING id")
        .bind(oid).bind(&b.reason).bind(&detail).bind(user).bind(guest).fetch_one(&st.pool).await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "id": id, "status": "open" })),
    )
        .into_response())
}

async fn create(
    State(st): State<AppState>,
    Path(slug): Path<String>,
    mut parts: Parts,
    Json(b): Json<NewReport>,
) -> Result<Response, AppError> {
    let detail = validate_body(&b.reason, b.detail.as_deref())?;
    if !crate::validate::slug_ok(&slug) {
        return Err(AppError::NotFound);
    }
    let w: Option<(Uuid, String, Uuid, String)> = sqlx::query_as(
        "SELECT id, moderation_status::text, owner_id, visibility::text FROM wishlists
         WHERE slug = $1 AND deleted_at IS NULL AND status IN ('active', 'closed')",
    )
    .bind(&slug)
    .fetch_optional(&st.pool)
    .await?;
    let (wid, m, owner_id, vis) = w.ok_or(AppError::NotFound)?;
    let viewer = crate::access::viewer(&parts, &st).await?;
    crate::access::check_wishlist(
        &st.pool,
        &crate::access::ListAccess {
            wishlist_id: wid,
            owner_id,
            visibility: &vis,
        },
        viewer,
    )
    .await?;
    if m == "hidden" {
        return Err(AppError::WishlistRemoved);
    }

    let (user, guest) = reporter(&mut parts, &st).await?;
    if user.is_some() || guest.is_some() {
        let dup: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM content_reports WHERE wishlist_id = $1 AND created_at > now() - interval '24 hours'
               AND reporter_user_id IS NOT DISTINCT FROM $2 AND reporter_guest_id IS NOT DISTINCT FROM $3 LIMIT 1")
            .bind(wid).bind(user).bind(guest).fetch_optional(&st.pool).await?;
        if let Some(id) = dup {
            return Ok((
                StatusCode::CREATED,
                Json(json!({ "id": id, "status": "open" })),
            )
                .into_response());
        }
    }
    let ip = crate::config::client_ip(&parts, &crate::config::get());
    ratelimit::check(&st.pool, &format!("report_ip:{ip}"), 10, 3600).await?;
    verify_turnstile(&crate::config::get(), b.turnstile_token.as_deref(), &ip).await?;
    if user.is_none() && guest.is_none() {
        // 匿名檢舉者以 IP 近似「同一檢舉者」：同清單 24 小時 1 筆
        ratelimit::check(&st.pool, &format!("report_wl:{wid}:{ip}"), 1, 86400).await?;
    }
    // 快照與 INSERT 同一句（單次往返）；只存圖片 key，輸出時才換 URL（見 snapshot_urls）。items 最多 50 筆。
    const ITEM_SNAP: &str = "jsonb_build_object('id', i.id, 'title', i.title, 'description', i.description, 'product_url', i.product_url,
        'image_key', CASE WHEN i.image_status = 'ready' THEN i.image_key END)";
    let id: Uuid = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "INSERT INTO content_reports (wishlist_id, item_id, reason, detail, reporter_user_id, reporter_guest_id, snapshot)
         SELECT $1, $2, $3::report_reason, $4, $5, $6,
           jsonb_build_object('title', w.title, 'description', w.description,
             'cover_image_key', CASE WHEN w.cover_image_status = 'ready' THEN w.cover_image_key END,
             'items', COALESCE((SELECT jsonb_agg({ITEM_SNAP} ORDER BY i.sort_order, i.id)
                 FROM (SELECT * FROM wishlist_items WHERE wishlist_id = w.id AND deleted_at IS NULL ORDER BY sort_order, id LIMIT 50) i), '[]'::jsonb),
             'item', (SELECT {ITEM_SNAP} FROM wishlist_items i WHERE i.id = $2 AND i.wishlist_id = w.id))
         FROM wishlists w WHERE w.id = $1 RETURNING id")))
        .bind(wid).bind(b.item_id).bind(&b.reason).bind(&detail).bind(user).bind(guest).fetch_one(&st.pool).await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "id": id, "status": "open" })),
    )
        .into_response())
}

// 目標物件的 JSON 片段：REPORT_SQL 與 targets 共用（別名 w / ow / ru / og）
macro_rules! wl_obj {
    () => {
        "CASE WHEN w.id IS NULL THEN NULL ELSE jsonb_build_object('id', w.id, 'slug', w.slug::text, 'title', w.title,
         'moderation_status', w.moderation_status::text,
         'owner', CASE WHEN ow.id IS NULL THEN NULL ELSE jsonb_build_object('id', ow.id, 'handle', ow.handle, 'display_name', ow.display_name) END) END"
    };
}
macro_rules! user_obj {
    () => {
        "CASE WHEN ru.id IS NULL THEN NULL ELSE jsonb_build_object('id', ru.id, 'handle', ru.handle, 'display_name', ru.display_name, 'suspended', ru.blocked_at IS NOT NULL) END"
    };
}
macro_rules! org_obj {
    () => {
        "CASE WHEN og.id IS NULL THEN NULL ELSE jsonb_build_object('id', og.id, 'slug', og.slug::text, 'display_name', og.display_name, 'verification_status', og.verification_status,
           'live_sites', (SELECT count(*) FROM wishlists lw WHERE lw.org_id = og.id AND lw.type = 'relief' AND lw.status = 'active' AND lw.deleted_at IS NULL)) END"
    };
}

const REPORT_SQL: &str = concat!(
    "SELECT r.id, jsonb_build_object('id', r.id, 'wishlist', ", wl_obj!(), ",
       'user', ", user_obj!(), ",
       'org', ", org_obj!(), ",
       'target_open_count', (SELECT count(*) FROM content_reports o WHERE o.status = 'open'
          AND ((r.wishlist_id IS NOT NULL AND o.wishlist_id = r.wishlist_id) OR (r.reported_user_id IS NOT NULL AND o.reported_user_id = r.reported_user_id)
            OR (r.reported_org_id IS NOT NULL AND o.reported_org_id = r.reported_org_id))),
       'item_id', r.item_id,
       'item', CASE WHEN it.id IS NULL THEN NULL ELSE jsonb_build_object('id', it.id, 'title', it.title, 'deleted', it.deleted_at IS NOT NULL) END,
       'snapshot', r.snapshot,
       'reporter_user', CASE WHEN rp.id IS NULL THEN NULL ELSE jsonb_build_object('id', rp.id, 'handle', rp.handle, 'display_name', rp.display_name) END,
       'reporter_stats', CASE WHEN r.reporter_user_id IS NULL AND r.reporter_guest_id IS NULL THEN NULL ELSE
          (SELECT jsonb_build_object('total', count(*), 'dismissed', count(*) FILTER (WHERE s.status = 'dismissed')) FROM content_reports s
            WHERE (r.reporter_user_id IS NOT NULL AND s.reporter_user_id = r.reporter_user_id)
               OR (r.reporter_guest_id IS NOT NULL AND s.reporter_guest_id = r.reporter_guest_id)) END,
       'reason', r.reason::text, 'detail', r.detail, 'status', r.status::text,
       'reporter', CASE WHEN r.reporter_user_id IS NOT NULL THEN 'user' WHEN r.reporter_guest_id IS NOT NULL THEN 'guest' ELSE 'anonymous' END,
       'resolution_note', r.resolution_note, 'handled_at', r.handled_at, 'created_at', r.created_at)
     FROM content_reports r LEFT JOIN wishlists w ON w.id = r.wishlist_id LEFT JOIN users ru ON ru.id = r.reported_user_id
     LEFT JOIN organizations og ON og.id = r.reported_org_id
     LEFT JOIN users ow ON ow.id = w.owner_id LEFT JOIN users rp ON rp.id = r.reporter_user_id
     LEFT JOIN wishlist_items it ON it.id = r.item_id AND it.wishlist_id = r.wishlist_id"
);

/// 快照只存 key；staff 輸出換成可開啟的 URL（圖片皆在公開 bucket，用既有 helper）。
/// shortcut: 被取代 / 刪除的舊圖物件若已從 bucket 清掉，URL 會 404（沒有另建簽章機制），需要時再保留圖檔。
fn snapshot_urls(v: &mut Value) {
    fn conv(o: &mut Value, key: &str, url: &str, f: impl Fn(&str) -> Option<String>) {
        if let Some(m) = o.as_object_mut() {
            let k = m.remove(key).and_then(|k| k.as_str().map(String::from));
            m.insert(
                url.into(),
                k.and_then(|k| f(&k)).map_or(Value::Null, Value::from),
            );
        }
    }
    let ready = |k: &str| crate::public::image_url(Some(k), "ready");
    let Some(s) = v.get_mut("snapshot").filter(|s| s.is_object()) else {
        return;
    };
    conv(s, "cover_image_key", "cover_image_url", ready);
    conv(s, "avatar_key", "avatar_url", |k| {
        crate::uploads::avatar_url(Some(k))
    });
    conv(s, "logo_key", "logo_url", |k| {
        Some(crate::uploads::S3::get().public_url(k))
    });
    if let Some(items) = s.get_mut("items").and_then(Value::as_array_mut) {
        for i in items {
            conv(i, "image_key", "image_url", ready);
        }
    }
    if let Some(i) = s.get_mut("item") {
        conv(i, "image_key", "image_url", ready);
    }
}

#[derive(Deserialize)]
struct ReportQ {
    cursor: Option<Uuid>,
    limit: Option<i64>,
    offset: Option<i64>,
    status: Option<String>,
    kind: Option<String>,
    reason: Option<String>,
    from: Option<String>,
    to: Option<String>,
    wishlist_id: Option<Uuid>,
    user_id: Option<Uuid>,
    org_id: Option<Uuid>,
}

type Day = Option<chrono::NaiveDate>;

/// 共用篩選驗證：status 預設 open；from / to 為 YYYY-MM-DD（含當日，台北時區）。
fn parse_filters(q: &ReportQ) -> Result<(String, Day, Day), AppError> {
    let status = q.status.clone().unwrap_or_else(|| "open".into());
    if !["open", "actioned", "dismissed"].contains(&status.as_str()) {
        return Err(AppError::invalid(
            "/status",
            "INVALID",
            "open | actioned | dismissed",
        ));
    }
    if q.kind
        .as_deref()
        .is_some_and(|k| !["wishlist", "user", "org"].contains(&k))
    {
        return Err(AppError::invalid(
            "/kind",
            "INVALID",
            "wishlist | user | org",
        ));
    }
    if q.reason.as_deref().is_some_and(|r| !REASONS.contains(&r)) {
        return Err(AppError::invalid("/reason", "INVALID", "reason 不合法"));
    }
    let day = |v: &Option<String>, p: &str| -> Result<Day, AppError> {
        v.as_deref()
            .map(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d"))
            .transpose()
            .map_err(|_| AppError::invalid(p, "INVALID", "YYYY-MM-DD"))
    };
    Ok((status, day(&q.from, "/from")?, day(&q.to, "/to")?))
}

// $1 status、$4 kind、$5 reason、$6 from、$7 to（list 的 $2 = cursor、$3 = limit；targets 的 $2 = offset、$3 = limit）
const FILTER_SQL: &str = "r.status = $1::report_status
   AND ($4::text IS NULL OR ($4 = 'wishlist' AND r.wishlist_id IS NOT NULL) OR ($4 = 'user' AND r.reported_user_id IS NOT NULL) OR ($4 = 'org' AND r.reported_org_id IS NOT NULL))
   AND ($5::report_reason IS NULL OR r.reason = $5::report_reason)
   AND ($6::date IS NULL OR r.created_at >= ($6::date)::timestamp AT TIME ZONE 'Asia/Taipei')
   AND ($7::date IS NULL OR r.created_at < (($7::date + 1)::timestamp AT TIME ZONE 'Asia/Taipei'))";

async fn list(
    State(st): State<AppState>,
    _s: Staff,
    Query(q): Query<ReportQ>,
) -> Result<Response, AppError> {
    let (status, from, to) = parse_filters(&q)?;
    let lim = q.limit.unwrap_or(20).clamp(1, 100);
    let rows: Vec<(Uuid, Value)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "{REPORT_SQL} WHERE {FILTER_SQL} AND ($2::uuid IS NULL OR r.id > $2)
           AND ($8::uuid IS NULL OR r.wishlist_id = $8) AND ($9::uuid IS NULL OR r.reported_user_id = $9) AND ($10::uuid IS NULL OR r.reported_org_id = $10)
         ORDER BY r.id LIMIT $3")))
        .bind(status).bind(q.cursor).bind(lim + 1).bind(q.kind.as_deref()).bind(q.reason.as_deref())
        .bind(from).bind(to).bind(q.wishlist_id).bind(q.user_id).bind(q.org_id).fetch_all(&st.pool).await?;
    let rows = rows
        .into_iter()
        .map(|(id, mut v)| {
            snapshot_urls(&mut v);
            (id, v)
        })
        .collect();
    Ok(page(rows, lim))
}

/// 依目標分組（件數多的在前），給 staff 排處理優先序。
async fn targets(
    State(st): State<AppState>,
    _s: Staff,
    Query(q): Query<ReportQ>,
) -> Result<Json<Value>, AppError> {
    let (status, from, to) = parse_filters(&q)?;
    let lim = q.limit.unwrap_or(20).clamp(1, 100);
    let offset = q.offset.unwrap_or(0).max(0);
    let mut rows: Vec<Value> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT jsonb_build_object(
           'kind', CASE WHEN g.wishlist_id IS NOT NULL THEN 'wishlist' WHEN g.reported_user_id IS NOT NULL THEN 'user' ELSE 'org' END,
           'wishlist', {wl}, 'user', {us}, 'org', {og},
           'count', g.n, 'reasons', g.reasons, 'first_at', g.first_at, 'latest_at', g.latest_at)
         FROM (SELECT r.wishlist_id, r.reported_user_id, r.reported_org_id, count(*) AS n,
                      array_agg(DISTINCT r.reason::text ORDER BY r.reason::text) AS reasons,
                      min(r.created_at) AS first_at, max(r.created_at) AS latest_at
                 FROM content_reports r WHERE {FILTER_SQL}
                GROUP BY r.wishlist_id, r.reported_user_id, r.reported_org_id) g
         LEFT JOIN wishlists w ON w.id = g.wishlist_id LEFT JOIN users ow ON ow.id = w.owner_id
         LEFT JOIN users ru ON ru.id = g.reported_user_id LEFT JOIN organizations og ON og.id = g.reported_org_id
         ORDER BY g.n DESC, g.latest_at DESC, COALESCE(g.wishlist_id, g.reported_user_id, g.reported_org_id)
         LIMIT $3 OFFSET $2",
        wl = wl_obj!(), us = user_obj!(), og = org_obj!())))
        .bind(status).bind(offset).bind(lim + 1).bind(q.kind.as_deref()).bind(q.reason.as_deref())
        .bind(from).bind(to).fetch_all(&st.pool).await?;
    let next = (rows.len() as i64 > lim).then(|| offset + lim);
    rows.truncate(lim as usize);
    Ok(Json(json!({ "data": rows, "next_offset": next })))
}

/// 使用者的處分紀錄：被檢舉次數（本人 + 其清單）與停權 / 解除停權。不含 IP / UA。
async fn moderation_history(
    State(st): State<AppState>,
    _s: Staff,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE id = $1)")
        .bind(id)
        .fetch_one(&st.pool)
        .await?;
    if !exists {
        return Err(AppError::NotFound);
    }
    const SCOPE: &str = "(r.reported_user_id = $1 OR r.wishlist_id IN (SELECT id FROM wishlists WHERE owner_id = $1))";
    let counts: Value = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT jsonb_build_object('total', count(*), 'open', count(*) FILTER (WHERE r.status = 'open'),
           'actioned', count(*) FILTER (WHERE r.status = 'actioned'), 'dismissed', count(*) FILTER (WHERE r.status = 'dismissed'))
         FROM content_reports r WHERE {SCOPE}")))
        .bind(id).fetch_one(&st.pool).await?;
    let recent: Vec<Value> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT jsonb_build_object('id', r.id, 'target', CASE WHEN r.reported_user_id IS NOT NULL THEN 'user' ELSE 'wishlist' END,
           'wishlist', CASE WHEN w.id IS NULL THEN NULL ELSE jsonb_build_object('id', w.id, 'slug', w.slug::text, 'title', w.title) END,
           'reason', r.reason::text, 'status', r.status::text, 'created_at', r.created_at)
         FROM content_reports r LEFT JOIN wishlists w ON w.id = r.wishlist_id
         WHERE {SCOPE} ORDER BY r.created_at DESC, r.id DESC LIMIT 20")))
        .bind(id).fetch_all(&st.pool).await?;
    let suspensions: Vec<Value> = sqlx::query_scalar(
        "SELECT jsonb_build_object('at', a.created_at, 'blocked', (a.diff->'after'->>'blocked')::boolean,
           'reason', a.diff->>'reason', 'staff', jsonb_build_object('id', s.id, 'display_name', s.display_name))
         FROM audit_logs a LEFT JOIN users s ON s.id = a.actor_id
         WHERE a.action = 'user.update' AND a.entity_id = $1
           AND (a.diff->'before'->>'blocked') IS DISTINCT FROM (a.diff->'after'->>'blocked')
         ORDER BY a.created_at DESC, a.id DESC LIMIT 20")
        .bind(id).fetch_all(&st.pool).await?;
    Ok(Json(
        json!({ "counts": counts, "recent": recent, "suspensions": suspensions }),
    ))
}

#[derive(Deserialize)]
struct Handle {
    status: String,
    note: Option<String>,
}

async fn handle(
    State(st): State<AppState>,
    Staff(sid): Staff,
    Path(id): Path<Uuid>,
    Json(b): Json<Handle>,
) -> Result<Json<Value>, AppError> {
    if b.status != "actioned" && b.status != "dismissed" {
        return Err(AppError::invalid(
            "/status",
            "INVALID",
            "actioned | dismissed",
        ));
    }
    let note = crate::validate::optional_note(b.note.as_deref(), "/note", 1000)?;
    let mut tx = st.pool.begin().await?;
    let (cur, reporter_id): (String, Option<Uuid>) = sqlx::query_as(
        "SELECT status::text, reporter_user_id FROM content_reports WHERE id = $1 FOR UPDATE",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(AppError::NotFound)?;
    if cur != "open" {
        return Err(AppError::problem(
            409,
            "INVALID_STATE_TRANSITION",
            "檢舉已處理",
        ));
    }
    sqlx::query("UPDATE content_reports SET status = $2::report_status, handled_by = $3, handled_at = now(), resolution_note = $4 WHERE id = $1")
        .bind(id).bind(&b.status).bind(sid).bind(&note).execute(&mut *tx).await?;
    crate::notify::enqueue_report_resolved(&mut *tx, &[reporter_id], &b.status).await?;
    audit(
        &mut *tx,
        sid,
        "report.handle",
        "content_reports",
        Some(id),
        json!({ "status": b.status, "note": note }),
    )
    .await?;
    let (_, mut v): (Uuid, Value) =
        sqlx::query_as(sqlx::AssertSqlSafe(format!("{REPORT_SQL} WHERE r.id = $1")))
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
    tx.commit().await?;
    snapshot_urls(&mut v);
    Ok(Json(v))
}

#[derive(Deserialize)]
struct Moderate {
    moderation_status: String,
    reason: Option<String>,
}

async fn moderate(
    State(st): State<AppState>,
    Staff(sid): Staff,
    Path(id): Path<Uuid>,
    Json(b): Json<Moderate>,
) -> Result<Json<Value>, AppError> {
    let hidden = match b.moderation_status.as_str() {
        "hidden" => true,
        "ok" => false,
        _ => {
            return Err(AppError::invalid(
                "/moderation_status",
                "INVALID",
                "ok | hidden",
            ))
        }
    };
    let reason = crate::validate::optional_note(b.reason.as_deref(), "/reason", 1000)?;
    let reason = reason.as_deref();
    if hidden && reason.is_none() {
        return Err(AppError::invalid("/reason", "REQUIRED", "下架必須填原因"));
    }
    let mut tx = st.pool.begin().await?;
    type ModerationRow = (
        String,
        String,
        String,
        Option<String>,
        Option<chrono::DateTime<chrono::Utc>>,
    );
    // 目標狀態與原因都沒變就不更新（F15：重送回 200，不重寫 moderated_at、不重複 audit / 通知）
    let row: Option<ModerationRow> = sqlx::query_as(
        "UPDATE wishlists SET moderation_status = $2::moderation_status, moderation_reason = $3, moderated_at = now(), moderated_by = $4
         WHERE id = $1 AND deleted_at IS NULL
           AND (moderation_status <> $2::moderation_status OR moderation_reason IS DISTINCT FROM $3)
         RETURNING id::text, slug::text, moderation_status::text, moderation_reason, moderated_at")
        .bind(id).bind(&b.moderation_status).bind(if hidden { reason } else { None }).bind(sid).fetch_optional(&mut *tx).await?;
    let Some((_, slug, ms, mr, at)) = row else {
        let (_, slug, ms, mr, at): ModerationRow = sqlx::query_as(
            "SELECT id::text, slug::text, moderation_status::text, moderation_reason, moderated_at FROM wishlists WHERE id = $1 AND deleted_at IS NULL")
            .bind(id).fetch_optional(&mut *tx).await?.ok_or(AppError::NotFound)?;
        return Ok(Json(
            json!({ "id": id, "slug": slug, "moderation_status": ms, "moderation_reason": mr, "moderated_at": at }),
        ));
    };
    // 只有實際改變才走到這裡（上面的條件式 UPDATE 已排除重送）
    crate::notify::enqueue_owner(
        &mut *tx,
        id,
        "wishlist.moderated",
        json!({"hidden": hidden, "reason": reason}),
    )
    .await?;
    audit(
        &mut *tx,
        sid,
        "wishlist.moderate",
        "wishlists",
        Some(id),
        json!({ "status": b.moderation_status, "reason": reason }),
    )
    .await?;
    if hidden {
        let reporters: Vec<Option<Uuid>> = sqlx::query_scalar("UPDATE content_reports SET status = 'actioned', handled_by = $2, handled_at = now(), resolution_note = $3 WHERE wishlist_id = $1 AND status = 'open' RETURNING reporter_user_id")
            .bind(id).bind(sid).bind(reason).fetch_all(&mut *tx).await?;
        crate::notify::enqueue_report_resolved(&mut *tx, &reporters, "actioned").await?;
    }
    dashboard::notify(&mut *tx, id).await?; // commit 時才送出，SSE 收到後關閉連線
    tx.commit().await?;
    if hidden {
        crate::cdn::purge_wishlists(vec![slug.clone()]);
    } // 下架：邊緣快取的公開副本立即清掉
    Ok(Json(
        json!({ "id": id, "slug": slug, "moderation_status": ms, "moderation_reason": mr, "moderated_at": at }),
    ))
}
