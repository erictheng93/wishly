//! 捐助：POST /items/{id}/claims、PATCH/DELETE /claims/{id}（契約 4.1 / 4.2、F4 / F5）。
//! 鎖序：wishlist_items（條件式 UPDATE / FOR UPDATE）→ claims。
use crate::{
    error::AppError,
    guest::{Actor, LegacyGuest, MaybeActor},
    idempotency, ratelimit,
    session::CurrentUser,
    AppState,
};
use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{patch, post},
    Router,
};
use chrono::{DateTime, Utc};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{FromRow, Postgres, Transaction};
use uuid::Uuid;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/items/{id}/claims", post(create))
        .route("/claims/{id}", patch(update).delete(cancel))
}

/// claims 以別名 `c` 查詢
pub const CLAIM_COLS: &str = "c.id, c.item_id, c.qty, c.status::text AS status, c.claimer_name, c.note, c.expires_at,
    c.created_at, c.updated_at, c.purchased_at, c.delivered_at, c.cancelled_at, c.visibility::text AS visibility, c.is_anonymous";

#[derive(FromRow, Serialize)]
pub struct ClaimRow {
    pub id: Uuid,
    pub item_id: Uuid,
    pub qty: i32,
    pub status: String,
    pub claimer_name: String,
    pub note: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub purchased_at: Option<DateTime<Utc>>,
    pub delivered_at: Option<DateTime<Utc>>,
    pub cancelled_at: Option<DateTime<Utc>>,
    /// 登入者捐助的公開層級（public/friends/private）；舊訪客捐助為 null
    pub visibility: Option<String>,
    pub is_anonymous: bool,
}

/// trim + 字元驗證；空（含只有零寬字元）→ None
fn clean(s: Option<&str>, ptr: &str, multi: bool) -> Result<Option<String>, AppError> {
    match s {
        Some(s) => Ok(Some(crate::validate::text(s, ptr, multi)?).filter(|s| !s.is_empty())),
        None => Ok(None),
    }
}

fn parse<T: DeserializeOwned>(b: &[u8]) -> Result<T, AppError> {
    serde_json::from_slice(b).map_err(|_| AppError::invalid("/", "INVALID", "請求內容格式錯誤"))
}

fn no_store(status: StatusCode) -> axum::http::response::Builder {
    Response::builder()
        .status(status)
        .header(header::CACHE_CONTROL, "private, no-store")
}

fn json_resp(status: i32, body: &Value, replay: bool) -> Response {
    let mut b = no_store(StatusCode::from_u16(status as u16).unwrap())
        .header(header::CONTENT_TYPE, "application/json");
    if status == 201 {
        if let Some(id) = body["claim"]["id"].as_str() {
            b = b.header(header::LOCATION, format!("/api/v1/claims/{id}"));
        }
    }
    if replay {
        b = b.header("Idempotency-Replayed", HeaderValue::from_static("true"));
    }
    b.body(axum::body::Body::from(body.to_string())).unwrap()
}

async fn audit(
    tx: &mut Transaction<'_, Postgres>,
    actor: Actor,
    action: &str,
    claim: Uuid,
    diff: Value,
) -> Result<(), AppError> {
    let (t, id) = match actor {
        Actor::Guest(g) => ("guest", g),
        Actor::User(u) => ("user", u),
    };
    sqlx::query("INSERT INTO audit_logs (actor_type, actor_id, action, entity, entity_id, diff) VALUES ($1::actor_type, $2, $3, 'claims', $4, $5)")
        .bind(t).bind(id).bind(action).bind(claim).bind(diff).execute(&mut **tx).await?;
    Ok(())
}

fn item_json(id: Uuid, needed: i32, claimed: i32) -> Value {
    json!({ "id": id, "qty_needed": needed, "qty_claimed": claimed, "qty_remaining": needed - claimed })
}

fn full(remaining: i32, want: i32) -> AppError {
    AppError::Extra {
        status: 409,
        code: "ITEM_FULLY_CLAIMED",
        detail: format!("此品項剩餘 {remaining} 件，無法再捐助 {want} 件。"),
        extra: json!({ "remaining": remaining }),
    }
}

// ---------- POST /items/{id}/claims ----------

#[derive(Deserialize)]
struct CreateReq {
    qty: i32,
    note: Option<String>,
    visibility: Option<String>,
    is_anonymous: Option<bool>,
}

pub(crate) fn share_level(v: &Option<String>) -> Result<(), AppError> {
    match v.as_deref() {
        None | Some("public" | "friends" | "private") => Ok(()),
        _ => Err(AppError::invalid(
            "/visibility",
            "ENUM",
            "visibility 只能是 public / friends / private",
        )),
    }
}

async fn create(
    State(st): State<AppState>,
    Path(item_id): Path<Uuid>,
    me: CurrentUser,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, AppError> {
    let key = idempotency::key(&headers)?;
    let req: CreateReq = parse(&body)?;
    if !(1..=99).contains(&req.qty) {
        return Err(AppError::invalid("/qty", "RANGE", "qty 必須介於 1 與 99"));
    }
    share_level(&req.visibility)?;
    let mut req = req;
    req.note = clean(req.note.as_deref(), "/note", true)?;
    if req.note.as_deref().is_some_and(|n| n.chars().count() > 200) {
        return Err(AppError::invalid("/note", "RANGE", "備註至多 200 字"));
    }

    // 存取判斷（看得到才能捐助）：好友/名單清單；品項不存在則交給後面的 diagnose 回 404
    let wl: Option<(Uuid, Uuid, String)> = sqlx::query_as(
        "SELECT w.id, w.owner_id, w.visibility::text FROM wishlist_items i JOIN wishlists w ON w.id = i.wishlist_id
         WHERE i.id = $1 AND i.deleted_at IS NULL AND w.deleted_at IS NULL").bind(item_id).fetch_optional(&st.pool).await?;
    if let Some((wid, owner_id, vis)) = &wl {
        let viewer = Some(me.id);
        crate::access::check_wishlist(
            &st.pool,
            &crate::access::ListAccess {
                wishlist_id: *wid,
                owner_id: *owner_id,
                visibility: vis,
            },
            viewer,
        )
        .await?;
    }

    let route = "POST /items/{id}/claims";
    let scope = format!("user:{}:{route}", me.id);
    let hash = idempotency::request_hash(&format!("POST /items/{item_id}/claims"), &body);

    let mut tx = st.pool.begin().await?;
    if let idempotency::Begin::Replay(s, b) =
        idempotency::begin(&mut tx, &scope, key, &hash).await?
    {
        return Ok(json_resp(s, &b, true));
    }
    // 限流放在重播判斷之後：重播只回放已存的回應、不產生新捐助，不消耗額度（也讓網路重試不會被誤擋）。
    // 超限時 tx 隨 return 回滾，idempotency key 不會被占用，額度重置後可用同 key 重試。
    // 計數與捐助同一個 tx（不另佔連線，避免併發時 pool 耗盡死結；429 時計數一併回滾，只算成功放行的請求）。
    // 以使用者計數，不以 IP：派對場地 Wi-Fi、電信商 CGNAT 會讓很多真人共用同一個 IP；每人每小時 30 次、同一清單 15 次。
    let w = format!("user:{}", me.id);
    ratelimit::check(&mut *tx, &format!("claim_post:{w}"), 30, 3600).await?;
    let wl_of: Option<Uuid> =
        sqlx::query_scalar("SELECT wishlist_id FROM wishlist_items WHERE id = $1")
            .bind(item_id)
            .fetch_optional(&mut *tx)
            .await?;
    if let Some(wl) = wl_of {
        ratelimit::check(&mut *tx, &format!("claim_post_wl:{wl}:{w}"), 15, 3600).await?;
    }

    // 使用者 → 清單：刪帳號先鎖 users，捐助的 FK 不能反向等待。
    let name: String = sqlx::query_scalar(
        "SELECT display_name FROM users WHERE id = $1 AND deleted_at IS NULL FOR SHARE",
    )
    .bind(me.id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(AppError::Unauthorized)?;

    // 清單 → 品項：與封存 / 強制刪除互斥，避免 UPDATE 的舊快照接受已封存清單。
    if let Some((wid, owner, ..)) = &wl {
        // 通知也引用清單擁有者；同樣要先鎖 users，避免與擁有者刪帳號互鎖。
        sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM users WHERE id = $1 AND deleted_at IS NULL FOR SHARE",
        )
        .bind(owner)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(AppError::NotFound)?;
        sqlx::query("SELECT id FROM wishlists WHERE id = $1 FOR SHARE")
            .bind(wid)
            .execute(&mut *tx)
            .await?;
    }
    // 4.1 (b)：條件式 UPDATE 取得 item row lock 並驗證剩餘量。
    // 偏差：加 moderation_status='ok' 與 visibility<>'private'，下架/私人清單不可捐助。
    let upd: Option<(i32, i32, Option<i32>)> = sqlx::query_as(
        "UPDATE wishlist_items i SET qty_claimed = i.qty_claimed + $2
         FROM wishlists w
         WHERE i.id = $1 AND w.id = i.wishlist_id
           AND w.status = 'active' AND (w.closes_at IS NULL OR w.closes_at > now()) AND w.deleted_at IS NULL AND w.moderation_status = 'ok' AND w.visibility <> 'private'
           AND i.deleted_at IS NULL AND i.funding_mode = 'quantity' AND i.qty_claimed + $2 <= i.qty_needed
         RETURNING i.qty_needed, i.qty_claimed, w.claim_ttl_hours")
        .bind(item_id).bind(req.qty).fetch_optional(&mut *tx).await?;
    let Some((needed, claimed, ttl)) = upd else {
        tx.rollback().await?;
        return Err(diagnose(&st, item_id, req.qty).await?);
    };

    let ins = sqlx::query_as::<_, ClaimRow>(sqlx::AssertSqlSafe(format!(
        "INSERT INTO claims AS c (item_id, user_id, claimer_name, qty, note, expires_at, visibility, is_anonymous)
         VALUES ($1, $2, $3, $4, $5, CASE WHEN $6::int IS NULL THEN NULL ELSE now() + $6::int * interval '1 hour' END, $7::text::share_level, $8)
         RETURNING {CLAIM_COLS}")))
        .bind(item_id).bind(me.id).bind(&name).bind(req.qty).bind(&req.note).bind(ttl)
        .bind(req.visibility.as_deref()) // NULL 時由 trigger 套帳號預設
        .bind(req.is_anonymous.unwrap_or(false))
        .fetch_one(&mut *tx).await;
    let claim = match ins {
        Ok(c) => c,
        Err(sqlx::Error::Database(e)) if e.code().as_deref() == Some("23505") => {
            drop(tx);
            let id: Option<Uuid> = sqlx::query_scalar(
                "SELECT id FROM claims WHERE item_id = $1 AND user_id = $2 AND status IN ('reserved','purchased','delivered')")
                .bind(item_id).bind(me.id).fetch_optional(&st.pool).await?;
            return Err(AppError::Extra {
                status: 409,
                code: "CLAIM_ALREADY_EXISTS",
                detail: "你已捐助過此品項，請改用修改數量".into(),
                extra: json!({ "claim_id": id }),
            });
        }
        Err(e) => return Err(e.into()),
    };

    audit(
        &mut tx,
        Actor::User(me.id),
        "claim.create",
        claim.id,
        json!({ "qty": req.qty }),
    )
    .await?;
    let resp = json!({ "claim": claim, "item": item_json(item_id, needed, claimed) });
    idempotency::finish(&mut tx, &scope, key, 201, &resp).await?;
    let wid = wishlist_of(&mut tx, item_id).await?;
    crate::dashboard::notify(&mut *tx, wid).await?;
    tx.commit().await?;
    Ok(json_resp(201, &resp, false))
}

/// 4.1 (d)：失敗歸因（rollback 後獨立查詢）
async fn diagnose(st: &AppState, item_id: Uuid, want: i32) -> Result<AppError, AppError> {
    let r: Option<(bool, String, String, String, String, i32)> = sqlx::query_as(
        "SELECT (i.deleted_at IS NOT NULL OR w.deleted_at IS NOT NULL), CASE WHEN w.status = 'active' AND w.closes_at <= now() THEN 'closed' ELSE w.status::text END, w.visibility::text, w.moderation_status::text,
                i.funding_mode::text, i.qty_needed - i.qty_claimed
         FROM wishlist_items i JOIN wishlists w ON w.id = i.wishlist_id WHERE i.id = $1")
        .bind(item_id).fetch_optional(&st.pool).await?;
    Ok(match r {
        None | Some((true, ..)) => AppError::NotFound,
        Some((_, _, _, m, ..)) if m == "hidden" => AppError::WishlistRemoved,
        Some((_, s, v, ..)) if s == "draft" || v == "private" => AppError::NotFound,
        Some((_, s, ..)) if s != "active" => {
            AppError::problem(409, "WISHLIST_CLOSED", "此清單已結束，不再接受捐助")
        }
        Some((_, _, _, _, f, _)) if f != "quantity" => {
            AppError::problem(409, "FUNDING_MODE_MISMATCH", "此品項不是數量型捐助")
        }
        Some((.., remaining)) => full(remaining, want),
    })
}

// ---------- PATCH / DELETE /claims/{id} ----------

#[derive(Deserialize)]
struct PatchReq {
    qty: Option<i32>,
    note: Option<String>,
    status: Option<String>,
    visibility: Option<String>,
    /// Q13：捐助者確認已看過品項修改（只記時間，不算狀態 / 數量變更）
    #[serde(default)]
    ack_item_change: bool,
}

async fn update(
    State(st): State<AppState>,
    Path(id): Path<Uuid>,
    MaybeActor(actor): MaybeActor,
    LegacyGuest(legacy): LegacyGuest,
    body: Bytes,
) -> Result<Response, AppError> {
    let p: PatchReq = parse(&body)?;
    if p.qty.is_none()
        && p.note.is_none()
        && p.status.is_none()
        && p.visibility.is_none()
        && !p.ack_item_change
    {
        return Err(AppError::invalid(
            "/",
            "REQUIRED",
            "至少需提供 qty、note、status、visibility 或 ack_item_change",
        ));
    }
    share_level(&p.visibility)?;
    if p.qty.is_some_and(|q| !(1..=99).contains(&q)) {
        return Err(AppError::invalid("/qty", "RANGE", "qty 必須介於 1 與 99"));
    }
    let mut p = p;
    if let Some(n) = p.note.as_deref() {
        p.note = Some(crate::validate::text(n, "/note", true)?);
    }
    if p.note.as_deref().is_some_and(|n| n.chars().count() > 200) {
        return Err(AppError::invalid("/note", "RANGE", "備註至多 200 字"));
    }
    if p.status
        .as_deref()
        .is_some_and(|s| !["purchased", "delivered", "cancelled"].contains(&s))
    {
        return Err(AppError::invalid(
            "/status",
            "ENUM",
            "status 只能是 purchased / delivered / cancelled",
        ));
    }
    let (claim, item) = apply(&st, id, actor, legacy, p).await?;
    Ok(json_resp(
        200,
        &json!({ "claim": claim, "item": item }),
        false,
    ))
}

/// 已 cancelled 的重複 DELETE 回 204（冪等）
async fn cancel(
    State(st): State<AppState>,
    Path(id): Path<Uuid>,
    MaybeActor(actor): MaybeActor,
    LegacyGuest(legacy): LegacyGuest,
) -> Result<Response, AppError> {
    apply(
        &st,
        id,
        actor,
        legacy,
        PatchReq {
            qty: None,
            note: None,
            status: Some("cancelled".into()),
            visibility: None,
            ack_item_change: false,
        },
    )
    .await?;
    Ok(no_store(StatusCode::NO_CONTENT)
        .body(axum::body::Body::empty())
        .unwrap()
        .into_response())
}

async fn apply(
    st: &AppState,
    id: Uuid,
    actor: Option<Actor>,
    legacy: Option<Uuid>,
    p: PatchReq,
) -> Result<(ClaimRow, Value), AppError> {
    let actor = actor.ok_or(AppError::Unauthorized)?;
    // 加量時的存取判斷比照 create 在交易外先做：check_wishlist 另取連線，放在交易內併發時會耗盡 pool
    let access = match (&actor, p.qty) {
        (Actor::User(u), Some(_)) => {
            let w: Option<(Uuid, Uuid, String)> = sqlx::query_as(
                "SELECT w.id, w.owner_id, w.visibility::text FROM claims c
                   JOIN wishlist_items i ON i.id = c.item_id JOIN wishlists w ON w.id = i.wishlist_id
                  WHERE c.id = $1",
            )
            .bind(id)
            .fetch_optional(&st.pool)
            .await?;
            match w {
                Some((wishlist_id, owner_id, vis)) => {
                    crate::access::check_wishlist(
                        &st.pool,
                        &crate::access::ListAccess {
                            wishlist_id,
                            owner_id,
                            visibility: &vis,
                        },
                        Some(*u),
                    )
                    .await
                }
                None => Ok(()),
            }
        }
        _ => Ok(()),
    };
    let mut tx = st.pool.begin().await?;
    // F5 交易邊界：先（不加鎖）取 item_id → 鎖 item → 鎖 claim
    let item_id: Uuid = sqlx::query_scalar("SELECT item_id FROM claims WHERE id = $1")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(AppError::NotFound)?;
    let (needed, claimed, wl_status, owner_id, locked, vis, hidden, gone, past_due): (
        i32,
        i32,
        String,
        Uuid,
        bool,
        String,
        bool,
        bool,
        bool,
    ) = sqlx::query_as(
        "SELECT i.qty_needed, i.qty_claimed, w.status::text, w.owner_id,
                surprise_locked(w.surprise_mode, w.event_date),
                w.visibility::text, w.moderation_status::text = 'hidden',
                (i.deleted_at IS NOT NULL OR w.deleted_at IS NOT NULL), (w.closes_at IS NOT NULL AND w.closes_at <= now())
         FROM wishlist_items i JOIN wishlists w ON w.id = i.wishlist_id
         WHERE i.id = $1 FOR UPDATE OF i").bind(item_id).fetch_one(&mut *tx).await?;
    let (guest_id, user_id, cur, old_qty): (Option<Uuid>, Option<Uuid>, String, i32) =
        sqlx::query_as(
            "SELECT guest_id, user_id, status::text, qty FROM claims WHERE id = $1 FOR UPDATE",
        )
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
    // 已登入會員若同時帶著舊訪客 token，也能管理自己登入前留下的訪客捐助
    let mine = match actor {
        Actor::Guest(g) => guest_id == Some(g),
        Actor::User(u) => user_id == Some(u) || (guest_id.is_some() && guest_id == legacy),
    };
    // 清單擁有者（F5）：只能推進 delivered 或取消；驚喜鎖定期間看不到捐助，一律禁止
    let by_owner = !mine && matches!(actor, Actor::User(u) if u == owner_id);
    // 非本人一律 404，不洩漏捐助存在（F5）
    if !mine && !by_owner {
        return Err(AppError::NotFound);
    }
    if by_owner
        && (locked
            || p.qty.is_some()
            || p.note.is_some()
            || p.visibility.is_some()
            || p.ack_item_change
            || !matches!(p.status.as_deref(), Some("delivered" | "cancelled")))
    {
        return Err(AppError::problem(
            403,
            "FORBIDDEN",
            "清單擁有者只能在驚喜解鎖後標記已送達或取消捐助",
        ));
    }

    if p.visibility.is_some() && user_id.is_none() {
        return Err(AppError::invalid(
            "/visibility",
            "NOT_APPLICABLE",
            "舊訪客捐助沒有公開層級設定",
        ));
    }
    let bad = || AppError::problem(409, "INVALID_STATE_TRANSITION", "不允許的狀態轉換");
    let mut new_status = cur.clone();
    if let Some(s) = p.status.as_deref() {
        match (cur.as_str(), s) {
            // 目標狀態與現況相同：冪等 no-op，回目前資料（F5）
            (a, b) if a == b => {}
            ("reserved", "purchased")
            | ("reserved" | "purchased", "delivered")
            | ("reserved" | "purchased", "cancelled") => new_status = s.into(),
            _ => return Err(bad()),
        }
    }
    // Q2：清單結束後捐助者只能取消，不能再推進為已購買 / 已送達；擁有者不受限（F8）
    let closed =
        matches!(wl_status.as_str(), "closed" | "archived") || (wl_status == "active" && past_due);
    if closed && !by_owner && new_status != cur && new_status != "cancelled" {
        return Err(AppError::problem(
            409,
            "WISHLIST_CLOSED",
            "此清單已結束，只能取消捐助",
        ));
    }
    let cancelling = new_status == "cancelled";
    let mut new_qty = None;
    if let (Some(q), false) = (p.qty, cancelling) {
        if !["reserved", "purchased"].contains(&cur.as_str()) {
            return Err(bad());
        }
        if wl_status != "active" {
            return Err(AppError::problem(
                409,
                "WISHLIST_CLOSED",
                "此清單已結束，無法修改數量",
            ));
        }
        let delta = q - old_qty;
        if delta > 0 && guest_id.is_some() {
            // 舊訪客捐助只保留管理權（減量 / 備註 / 取消）；加量等同新認捐，需以會員身分另行認捐
            return Err(match actor {
                Actor::Guest(_) => AppError::Unauthorized,
                Actor::User(_) => AppError::problem(
                    403,
                    "LEGACY_CLAIM_READONLY",
                    "舊訪客捐助無法增加數量，請以會員身分另行認捐",
                ),
            });
        }
        if delta > 0 {
            // 加量等同新認捐：套用 create 的同一組閘門（刪除 / 審核下架 / 私人 / 截止 / 看得到才能捐）
            if gone || wl_status == "draft" || vis == "private" {
                return Err(AppError::NotFound);
            }
            if hidden {
                return Err(AppError::WishlistRemoved);
            }
            if past_due {
                return Err(AppError::problem(
                    409,
                    "WISHLIST_CLOSED",
                    "此清單已結束，不再接受捐助",
                ));
            }
            access?;
        }
        if delta != 0 {
            let r: Option<(i32,)> = sqlx::query_as("UPDATE wishlist_items SET qty_claimed = qty_claimed + $2 WHERE id = $1 AND qty_claimed + $2 <= qty_needed RETURNING qty_claimed")
                .bind(item_id).bind(delta).fetch_optional(&mut *tx).await?;
            if r.is_none() {
                return Err(full(needed - claimed, delta));
            }
            new_qty = Some(q);
        }
    }
    if cancelling && cur != "cancelled" {
        let r = sqlx::query("UPDATE wishlist_items SET qty_claimed = qty_claimed - $2 WHERE id = $1 AND qty_claimed >= $2")
            .bind(item_id).bind(old_qty).execute(&mut *tx).await?;
        if r.rows_affected() == 0 {
            tracing::error!(%id, "qty_claimed drift");
            return Err(AppError::Db(sqlx::Error::RowNotFound));
        }
    }
    if p.ack_item_change {
        sqlx::query("UPDATE claims SET item_change_seen_at = now() WHERE id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    let claim: ClaimRow = if new_status == cur
        && new_qty.is_none()
        && p.note.is_none()
        && p.visibility.is_none()
    {
        sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {CLAIM_COLS} FROM claims c WHERE c.id = $1"
        )))
        .bind(id)
        .fetch_one(&mut *tx)
        .await?
    } else {
        sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "UPDATE claims c SET qty = COALESCE($2, qty), note = COALESCE($3, note), visibility = COALESCE($5::text::share_level, visibility), status = $4::text::claim_status,
               purchased_at = CASE WHEN $4::text = 'purchased' AND c.status::text <> 'purchased' THEN now() ELSE purchased_at END,
               delivered_at = CASE WHEN $4::text = 'delivered' AND c.status::text <> 'delivered' THEN now() ELSE delivered_at END,
               cancelled_at = CASE WHEN $4::text = 'cancelled' AND c.status::text <> 'cancelled' THEN now() ELSE cancelled_at END,
               expires_at = CASE WHEN $4::text <> 'reserved' THEN NULL ELSE expires_at END
             WHERE c.id = $1 RETURNING {CLAIM_COLS}")))
            .bind(id).bind(new_qty).bind(&p.note).bind(&new_status).bind(&p.visibility).fetch_one(&mut *tx).await?
    };
    let (n, c): (i32, i32) =
        sqlx::query_as("SELECT qty_needed, qty_claimed FROM wishlist_items WHERE id = $1")
            .bind(item_id)
            .fetch_one(&mut *tx)
            .await?;
    if new_status != cur || new_qty.is_some() {
        let action = if by_owner {
            "claim.owner_update"
        } else {
            "claim.update"
        };
        audit(
            &mut tx,
            actor,
            action,
            id,
            json!({ "status": [cur, new_status], "qty": [old_qty, claim.qty] }),
        )
        .await?;
    }
    let wid = wishlist_of(&mut tx, item_id).await?;
    crate::dashboard::notify(&mut *tx, wid).await?;
    tx.commit().await?;
    Ok((claim, item_json(item_id, n, c)))
}

async fn wishlist_of(tx: &mut Transaction<'_, Postgres>, item_id: Uuid) -> Result<Uuid, AppError> {
    Ok(
        sqlx::query_scalar("SELECT wishlist_id FROM wishlist_items WHERE id = $1")
            .bind(item_id)
            .fetch_one(&mut **tx)
            .await?,
    )
}

/// FR-08：把逾期的 reserved 捐助設為 expired 並回補 qty_claimed，回傳釋放的捐助數。
/// 鎖序同 apply()：先鎖 item（依 id 排序）再動 claim；UPDATE ... WHERE status='reserved' 保證多實例 / 與取消競態時只回補一次。
pub async fn expire_due(pool: &sqlx::PgPool) -> Result<usize, sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT id FROM wishlist_items WHERE id IN (SELECT item_id FROM claims WHERE status = 'reserved' AND expires_at <= now())
                 ORDER BY id FOR UPDATE").execute(&mut *tx).await?;
    let rows: Vec<(Uuid, i64)> = sqlx::query_as(
        "WITH x AS (
           UPDATE claims SET status = 'expired', expires_at = NULL
           WHERE status = 'reserved' AND expires_at <= now() RETURNING id, item_id, qty),
         r AS (
           UPDATE wishlist_items i SET qty_claimed = i.qty_claimed - s.q
           FROM (SELECT item_id, sum(qty)::int AS q FROM x GROUP BY item_id) s
           WHERE i.id = s.item_id RETURNING i.wishlist_id),
         a AS (
           INSERT INTO audit_logs (actor_type, action, entity, entity_id, diff)
           SELECT 'system'::actor_type, 'claim.expire', 'claims', id, jsonb_build_object('qty', qty) FROM x)
         SELECT DISTINCT wishlist_id, (SELECT count(*) FROM x) FROM r")
        .fetch_all(&mut *tx).await?;
    for (w, _) in &rows {
        crate::dashboard::notify(&mut *tx, *w).await?;
    }
    tx.commit().await?;
    Ok(rows.first().map_or(0, |r| r.1 as usize))
}
