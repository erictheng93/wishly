//! 好友：邀請連結（/invite/{token}）、申請＋同意、列表、解除。見 0005_social。
use crate::{
    access::are_friends,
    error::AppError,
    guest::new_token,
    notify, ratelimit,
    session::{hash_token, CurrentUser},
    AppState,
};
use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{delete, get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

type R<T> = Result<T, AppError>;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/friends", get(list))
        .route("/friends/feed", get(feed))
        .route("/friends/requests", post(request))
        .route("/friends/search", get(search))
        .route("/friends/requests/count", get(request_count))
        .route("/friends/requests/{id}/accept", post(accept))
        .route("/friends/requests/{id}", delete(drop_request))
        .route(
            "/friends/invites",
            post(invite_create).delete(invite_revoke),
        )
        .route("/friends/invites/{token}", get(invite_preview))
        .route("/friends/invites/{token}/accept", post(invite_accept))
        .route("/friends/{user_id}", delete(unfriend))
}

fn pair(a: Uuid, b: Uuid) -> (Uuid, Uuid) {
    if a < b {
        (a, b)
    } else {
        (b, a)
    }
}

/// 好友配對的 advisory lock（交易結束自動釋放）。
/// 鎖序規則：凡使用它的交易，一律**最先**取這把鎖，之後才能碰任何列鎖（friend_requests / friendships 等），
/// 否則封鎖與申請 / 接受 / 邀請之間可能交錯而讓封鎖後仍留下好友關係。
pub(crate) async fn lock_pair(ex: &mut sqlx::PgConnection, a: Uuid, b: Uuid) -> R<()> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('friend_pair:' || least($1::uuid, $2::uuid)::text || ':' || greatest($1::uuid, $2::uuid)::text, 0))")
        .bind(a)
        .bind(b)
        .execute(&mut *ex)
        .await?;
    Ok(())
}

/// 測試用：模擬封鎖交易持有 pair lock（併發測試不能直接呼叫 pub(crate)）
#[doc(hidden)]
pub async fn lock_pair_for_test(ex: &mut sqlx::PgConnection, a: Uuid, b: Uuid) {
    lock_pair(ex, a, b).await.unwrap();
}

/// 任一方向有封鎖（對稱）
pub(crate) async fn blocked_between<'e, E: sqlx::PgExecutor<'e>>(
    ex: E,
    a: Uuid,
    b: Uuid,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM user_blocks WHERE (blocker_id = $1 AND blocked_id = $2) OR (blocker_id = $2 AND blocked_id = $1))")
        .bind(a)
        .bind(b)
        .fetch_one(ex)
        .await
}

/// 成為好友（冪等）；同時清掉雙方之間的待處理申請
async fn befriend(ex: &mut sqlx::PgConnection, a: Uuid, b: Uuid) -> R<()> {
    let (x, y) = pair(a, b);
    sqlx::query("INSERT INTO friendships (user_a, user_b) VALUES ($1, $2) ON CONFLICT DO NOTHING")
        .bind(x)
        .bind(y)
        .execute(&mut *ex)
        .await?;
    sqlx::query("DELETE FROM friend_requests WHERE (from_user = $1 AND to_user = $2) OR (from_user = $2 AND to_user = $1)").bind(a).bind(b).execute(&mut *ex).await?;
    Ok(())
}

async fn friend_json(ex: &mut sqlx::PgConnection, me: Uuid, other: Uuid) -> R<Value> {
    let (x, y) = pair(me, other);
    let (name, handle, since): (String, Option<String>, chrono::DateTime<chrono::Utc>) = sqlx::query_as(
        "SELECT u.display_name, u.handle, f.created_at FROM friendships f JOIN users u ON u.id = $3
         WHERE f.user_a = $1 AND f.user_b = $2").bind(x).bind(y).bind(other).fetch_one(&mut *ex).await?;
    Ok(json!({ "id": other, "display_name": name, "handle": handle, "since": since }))
}

async fn list(u: CurrentUser, State(st): State<AppState>) -> R<Json<Value>> {
    let friends: Vec<(Uuid, String, Option<String>, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
        "SELECT o.id, o.display_name, o.handle, f.created_at FROM friendships f
         JOIN users o ON o.id = CASE WHEN f.user_a = $1 THEN f.user_b ELSE f.user_a END AND o.deleted_at IS NULL
         WHERE f.user_a = $1 OR f.user_b = $1 ORDER BY o.display_name, o.id").bind(u.id).fetch_all(&st.pool).await?;
    let reqs = |incoming: bool| {
        let sql = format!("SELECT r.id, o.id, o.display_name, o.handle, r.created_at FROM friend_requests r
            JOIN users o ON o.id = r.{} AND o.deleted_at IS NULL WHERE r.{} = $1 ORDER BY r.created_at DESC",
            if incoming { "from_user" } else { "to_user" }, if incoming { "to_user" } else { "from_user" });
        let pool = st.pool.clone();
        async move {
            type FriendRequestRow = (
                Uuid,
                Uuid,
                String,
                Option<String>,
                chrono::DateTime<chrono::Utc>,
            );
            let rows: Vec<FriendRequestRow> = sqlx::query_as(sqlx::AssertSqlSafe(sql))
                .bind(u.id)
                .fetch_all(&pool)
                .await?;
            Ok::<_, AppError>(rows.into_iter().map(|(id, uid, n, h, at)| json!({ "id": id, "user": { "id": uid, "display_name": n, "handle": h }, "created_at": at })).collect::<Vec<_>>())
        }
    };
    Ok(Json(json!({
        "friends": friends.into_iter().map(|(id, n, h, s)| json!({ "id": id, "display_name": n, "handle": h, "since": s })).collect::<Vec<_>>(),
        "incoming": reqs(true).await?, "outgoing": reqs(false).await? })))
}

#[derive(Deserialize)]
struct ReqBody {
    to: String,
}

/// `to`：Email、@handle 或使用者 ID（搜尋結果送的是 ID）。找不到人回 404 USER_NOT_FOUND，
/// 成功回 202 `{ status: "requested" | "friends" }`（對方先邀過你就直接成為好友）
async fn request(
    u: CurrentUser,
    State(st): State<AppState>,
    Json(b): Json<ReqBody>,
) -> R<(StatusCode, Json<Value>)> {
    ratelimit::check(&st.pool, &format!("friend_req:{}", u.id), 30, 3600).await?;
    let to = b.to.trim().to_lowercase();
    if to.is_empty() || to.chars().count() > 200 {
        return Err(AppError::invalid(
            "/to",
            "FORMAT",
            "請輸入 Email 或帳號代號",
        ));
    }
    // Q9：也接受使用者 ID（門檻頁送邀請用）；handle 不含 '-'，不會與 UUID 混淆
    let target: Option<Uuid> = if let Ok(id) = Uuid::parse_str(&to) {
        sqlx::query_scalar("SELECT id FROM users WHERE id = $1 AND deleted_at IS NULL")
            .bind(id)
            .fetch_optional(&st.pool)
            .await?
    } else if to.contains('@') && !to.starts_with('@') {
        sqlx::query_scalar("SELECT id FROM users WHERE email = $1 AND deleted_at IS NULL")
            .bind(&to)
            .fetch_optional(&st.pool)
            .await?
    } else {
        sqlx::query_scalar("SELECT id FROM users WHERE handle = $1 AND deleted_at IS NULL")
            .bind(to.trim_start_matches('@'))
            .fetch_optional(&st.pool)
            .await?
    };
    let Some(t) = target else {
        return Err(AppError::problem(404, "USER_NOT_FOUND", "找不到這位使用者"));
    };
    if t == u.id {
        return Err(AppError::invalid("/to", "SELF", "不能加自己為好友"));
    }
    if are_friends(&st.pool, u.id, t).await? {
        return Ok((StatusCode::ACCEPTED, Json(json!({ "status": "friends" }))));
    }
    let mut tx = st.pool.begin().await?;
    lock_pair(&mut tx, u.id, t).await?;
    if blocked_between(&mut *tx, u.id, t).await? {
        // 與找不到人同一個回應，不洩漏封鎖
        return Err(AppError::problem(404, "USER_NOT_FOUND", "找不到這位使用者"));
    }
    let reverse: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM friend_requests WHERE from_user = $1 AND to_user = $2)",
    )
    .bind(t)
    .bind(u.id)
    .fetch_one(&mut *tx)
    .await?;
    if reverse {
        befriend(&mut tx, u.id, t).await?;
    } else {
        let new_id: Option<Uuid> = sqlx::query_scalar("INSERT INTO friend_requests (from_user, to_user) VALUES ($1, $2) ON CONFLICT DO NOTHING RETURNING id").bind(u.id).bind(t).fetch_optional(&mut *tx).await?;
        if let Some(rid) = new_id {
            notify::enqueue_friend_request(&mut *tx, t, u.id, rid).await?;
        }
    }
    tx.commit().await?;
    let status = if reverse { "friends" } else { "requested" };
    Ok((StatusCode::ACCEPTED, Json(json!({ "status": status }))))
}

#[derive(Deserialize)]
struct SearchQuery {
    q: String,
}

/// 加好友的邊打邊搜：全站依名稱（包含）、handle（前綴）、Email（完全相符）找人；輸入 Email 時只做完全相符。
/// 排序：完全相符 → 自己 / 好友 → 共同好友多者 → 名稱。
// shortcut: ILIKE '%q%' 是全表掃描，使用者量大到搜尋變慢時再加 pg_trgm GIN 索引
async fn search(
    u: CurrentUser,
    State(st): State<AppState>,
    Query(p): Query<SearchQuery>,
) -> R<Json<Value>> {
    ratelimit::check(&st.pool, &format!("user_search:{}", u.id), 600, 3600).await?;
    let q = p.q.trim().to_lowercase();
    if q.is_empty() || q.chars().count() > 100 {
        return Ok(Json(json!({ "users": [] })));
    }
    let esc = |s: &str| {
        s.replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
    };
    let bare = q.trim_start_matches('@');
    // 打的是 Email（中間有 @）就只做完全相符，不做部分比對，避免邊打邊試出別人的信箱。
    // 名稱含 @（多半是把信箱當名字）的人同理不參與部分比對，只有打出完整名稱才找得到。
    let partial = !(q.contains('@') && !q.starts_with('@'));
    #[allow(clippy::type_complexity)]
    let rows: Vec<(Uuid, String, Option<String>, Option<String>, String, i64)> = sqlx::query_as(
        "WITH mine AS (SELECT CASE WHEN user_a = $1 THEN user_b ELSE user_a END AS id FROM friendships WHERE $1 IN (user_a, user_b))
         SELECT u.id, u.display_name, u.handle, u.avatar_key,
                CASE WHEN u.id = $1 THEN 'self'
                     WHEN u.id IN (SELECT id FROM mine) THEN 'friend'
                     WHEN EXISTS (SELECT 1 FROM friend_requests WHERE from_user = $1 AND to_user = u.id) THEN 'outgoing'
                     WHEN EXISTS (SELECT 1 FROM friend_requests WHERE from_user = u.id AND to_user = $1) THEN 'incoming'
                     ELSE 'none' END AS relation,
                (SELECT count(*) FROM friendships f
                  WHERE (f.user_a = u.id AND f.user_b IN (SELECT id FROM mine))
                     OR (f.user_b = u.id AND f.user_a IN (SELECT id FROM mine))) AS mutual
           FROM users u
          WHERE u.deleted_at IS NULL AND u.blocked_at IS NULL
            AND NOT EXISTS (SELECT 1 FROM user_blocks b WHERE (b.blocker_id = $1 AND b.blocked_id = u.id) OR (b.blocker_id = u.id AND b.blocked_id = $1))
            AND ((u.display_name NOT LIKE '%@%' AND u.display_name ILIKE $2 ESCAPE '\\')
                 OR lower(u.display_name) = $4 OR u.handle LIKE $3 ESCAPE '\\' OR u.email = $4)
          ORDER BY (u.handle = $5 OR u.email = $4) IS TRUE DESC, -- handle / email 為 NULL 時比較結果是 NULL，DESC 會排到最前
                   (u.id = $1 OR u.id IN (SELECT id FROM mine)) DESC,
                   mutual DESC, u.display_name
          LIMIT 8",
    )
    .bind(u.id)
    .bind(partial.then(|| format!("%{}%", esc(&q))))
    .bind(partial.then(|| format!("{}%", esc(bare))))
    .bind(&q)
    .bind(bare)
    .fetch_all(&st.pool)
    .await?;
    let users: Vec<Value> = rows
        .into_iter()
        .map(|(id, n, h, a, rel, m)| {
            json!({
                "id": id, "display_name": n, "handle": h,
                "avatar_url": crate::uploads::avatar_url(a.as_deref()),
                "relation": rel, "mutual": m,
            })
        })
        .collect();
    Ok(Json(json!({ "users": users })))
}

/// 導覽列徽章用：待我處理的好友申請數（與 `list` 的 incoming 同一過濾）
async fn request_count(u: CurrentUser, State(st): State<AppState>) -> R<Json<Value>> {
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM friend_requests r JOIN users o ON o.id = r.from_user AND o.deleted_at IS NULL WHERE r.to_user = $1")
        .bind(u.id).fetch_one(&st.pool).await?;
    Ok(Json(json!({ "incoming": n })))
}

async fn accept(
    u: CurrentUser,
    State(st): State<AppState>,
    Path(id): Path<Uuid>,
) -> R<Json<Value>> {
    let mut tx = st.pool.begin().await?;
    // 先不加鎖讀出對方，取 pair lock 後才鎖列（鎖序：pair lock 最先）
    let peer: Uuid =
        sqlx::query_scalar("SELECT from_user FROM friend_requests WHERE id = $1 AND to_user = $2")
            .bind(id)
            .bind(u.id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(AppError::NotFound)?;
    lock_pair(&mut tx, u.id, peer).await?;
    if blocked_between(&mut *tx, u.id, peer).await? {
        return Err(AppError::NotFound);
    }
    let from: Uuid = sqlx::query_scalar(
        "SELECT from_user FROM friend_requests WHERE id = $1 AND to_user = $2 FOR UPDATE",
    )
    .bind(id)
    .bind(u.id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(AppError::NotFound)?;
    befriend(&mut tx, u.id, from).await?;
    let friend = friend_json(&mut tx, u.id, from).await?;
    crate::notify::enqueue(
        &mut *tx,
        from,
        "friend.accepted",
        json!({"from_user": u.id}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({ "friend": friend })))
}

async fn drop_request(
    u: CurrentUser,
    State(st): State<AppState>,
    Path(id): Path<Uuid>,
) -> R<StatusCode> {
    let n = sqlx::query(
        "DELETE FROM friend_requests WHERE id = $1 AND (to_user = $2 OR from_user = $2)",
    )
    .bind(id)
    .bind(u.id)
    .execute(&st.pool)
    .await?
    .rows_affected();
    if n == 0 {
        return Err(AppError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn unfriend(
    u: CurrentUser,
    State(st): State<AppState>,
    Path(other): Path<Uuid>,
) -> R<StatusCode> {
    let (x, y) = pair(u.id, other);
    let mut tx = st.pool.begin().await?;
    sqlx::query("DELETE FROM friendships WHERE user_a = $1 AND user_b = $2")
        .bind(x)
        .bind(y)
        .execute(&mut *tx)
        .await?;
    // 雙方清單的指定名單中，彼此都要移除
    sqlx::query("DELETE FROM wishlist_allowed_users a USING wishlists w
                 WHERE a.wishlist_id = w.id AND ((w.owner_id = $1 AND a.user_id = $2) OR (w.owner_id = $2 AND a.user_id = $1))")
        .bind(u.id).bind(other).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------- 邀請連結 ----------

#[derive(Deserialize)]
struct InviteBody {
    wishlist_id: Option<Uuid>,
}

#[derive(Deserialize)]
struct InviteQ {
    wishlist_id: Option<Uuid>,
}

/// body 可省略（舊前端不帶）；有內容才解析
async fn invite_create(
    u: CurrentUser,
    State(st): State<AppState>,
    body: Bytes,
) -> R<(StatusCode, Json<Value>)> {
    let wid = if body.iter().all(u8::is_ascii_whitespace) {
        None
    } else {
        serde_json::from_slice::<InviteBody>(&body)
            .map_err(|_| AppError::problem(400, "BAD_REQUEST", "body 格式錯誤"))?
            .wishlist_id
    };
    let list = match wid {
        None => None,
        Some(w) => {
            let (slug, title, vis): (String, String, String) = sqlx::query_as("SELECT slug::text, title, visibility::text FROM wishlists WHERE id = $1 AND owner_id = $2 AND deleted_at IS NULL")
                .bind(w).bind(u.id).fetch_optional(&st.pool).await?.ok_or(AppError::NotFound)?;
            if vis != "friends" && vis != "selected" {
                return Err(AppError::problem(
                    422,
                    "LIST_NOT_RESTRICTED",
                    "只有好友或指定好友可見的清單能附在邀請裡",
                ));
            }
            Some(json!({ "id": w, "slug": slug, "title": title }))
        }
    };
    let (token, h) = new_token();
    let mut tx = st.pool.begin().await?;
    // 一般邀請與每份清單各一條有效邀請，互不撤銷
    sqlx::query("UPDATE friend_invites SET revoked_at = now() WHERE user_id = $1 AND revoked_at IS NULL AND wishlist_id IS NOT DISTINCT FROM $2")
        .bind(u.id).bind(wid).execute(&mut *tx).await?;
    let exp: chrono::DateTime<chrono::Utc> = sqlx::query_scalar("INSERT INTO friend_invites (user_id, token_hash, wishlist_id, expires_at) VALUES ($1, $2, $3, now() + interval '7 days') RETURNING expires_at")
        .bind(u.id).bind(h).bind(wid).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(
            json!({ "token": token, "url": format!("{}/invite/{token}", crate::auth::app_url()), "expires_at": exp, "wishlist": list }),
        ),
    ))
}

/// 不帶 wishlist_id：撤銷全部有效邀請；帶：只撤銷該清單的
async fn invite_revoke(
    u: CurrentUser,
    State(st): State<AppState>,
    Query(q): Query<InviteQ>,
) -> R<StatusCode> {
    sqlx::query("UPDATE friend_invites SET revoked_at = now() WHERE user_id = $1 AND revoked_at IS NULL AND ($2::uuid IS NULL OR wishlist_id = $2)")
        .bind(u.id).bind(q.wishlist_id).execute(&st.pool).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// 有效邀請 → (邀請者 id, 暱稱, handle, 附帶清單 id)
async fn find_invite(
    pool: &PgPool,
    token: &str,
) -> R<(Uuid, String, Option<String>, Option<Uuid>)> {
    sqlx::query_as("SELECT u.id, u.display_name, u.handle, i.wishlist_id FROM friend_invites i JOIN users u ON u.id = i.user_id AND u.deleted_at IS NULL
                    WHERE i.token_hash = $1 AND i.revoked_at IS NULL AND i.expires_at > now()")
        .bind(hash_token(token)).fetch_optional(pool).await?.ok_or(AppError::NotFound)
}

/// 邀請附帶的清單（仍可從邀請進入者）→ (slug, title, visibility)；否則 None，邀請退化成一般好友邀請
async fn invite_list(
    pool: impl sqlx::PgExecutor<'_>,
    wid: Option<Uuid>,
    inviter: Uuid,
) -> R<Option<(String, String, String)>> {
    let Some(wid) = wid else { return Ok(None) };
    Ok(sqlx::query_as("SELECT slug::text, title, visibility::text FROM wishlists
                       WHERE id = $1 AND owner_id = $2 AND deleted_at IS NULL AND status::text IN ('active', 'closed')
                         AND moderation_status::text = 'ok' AND visibility::text IN ('friends', 'selected')")
        .bind(wid).bind(inviter).fetch_optional(pool).await?)
}

async fn invite_preview(State(st): State<AppState>, Path(token): Path<String>) -> R<Json<Value>> {
    let (inviter, n, h, wid) = find_invite(&st.pool, &token).await?;
    let list = invite_list(&st.pool, wid, inviter).await?;
    Ok(Json(json!({
        "inviter": { "display_name": n, "handle": h },
        "wishlist": list.map(|(slug, title, visibility)| json!({ "slug": slug, "title": title, "visibility": visibility })) })))
}

async fn invite_accept(
    u: CurrentUser,
    State(st): State<AppState>,
    Path(token): Path<String>,
) -> R<Json<Value>> {
    let (inviter, _, _, wid) = find_invite(&st.pool, &token).await?;
    if inviter == u.id {
        return Err(AppError::problem(422, "SELF_INVITE", "不能接受自己的邀請"));
    }
    let mut tx = st.pool.begin().await?;
    lock_pair(&mut tx, u.id, inviter).await?;
    if blocked_between(&mut *tx, u.id, inviter).await? {
        return Err(AppError::NotFound); // 與無效邀請同一個回應
    }
    befriend(&mut tx, u.id, inviter).await?;
    let list = invite_list(&mut *tx, wid, inviter).await?;
    // 產品決定：接受 selected 清單的邀請即自動加入該清單名單
    if let (Some(w), Some((_, _, vis))) = (wid, &list) {
        if vis == "selected" {
            sqlx::query("INSERT INTO wishlist_allowed_users (wishlist_id, user_id) VALUES ($1, $2) ON CONFLICT DO NOTHING")
                .bind(w).bind(u.id).execute(&mut *tx).await?;
        }
    }
    let friend = friend_json(&mut tx, u.id, inviter).await?;
    crate::notify::enqueue(
        &mut *tx,
        inviter,
        "friend.accepted",
        json!({"from_user": u.id}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(
        json!({ "friend": friend, "wishlist": list.map(|(slug, ..)| json!({ "slug": slug })) }),
    ))
}

#[derive(sqlx::FromRow)]
struct FeedList {
    slug: String,
    title: String,
    is_new: bool,
    event_date: Option<chrono::NaiveDate>,
    closes_at: Option<chrono::DateTime<chrono::Utc>>,
    owner_id: Uuid,
    display_name: String,
    handle: Option<String>,
    n_new: i64,
    at: chrono::DateTime<chrono::Utc>,
    n: i64,
    got: i64,
    need: i64,
    item_title: String,
    image_key: Option<String>,
    image_status: String,
    unit_price_amount: Option<i64>,
    open: bool,
}

/// 好友動態：近 30 天好友清單新增的品項（每份清單一則）＋ 我送出的禮物被標記送達；另附 30 天內的好友生日。
/// 清單可見性與 access::check_wishlist 同規則（private 不列、selected 需在名單），只列上線中、未截止的清單。
async fn feed(u: CurrentUser, State(st): State<AppState>) -> R<Json<Value>> {
    let lists: Vec<FeedList> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "WITH fr AS (SELECT CASE WHEN user_a = $1 THEN user_b ELSE user_a END AS id FROM friendships WHERE user_a = $1 OR user_b = $1)
         SELECT w.slug::text AS slug, w.title, w.created_at > now() - interval '30 days' AS is_new, w.event_date, w.closes_at,
                o.id AS owner_id, o.display_name, o.handle,
                s.n_new, s.at, s.n, s.got, s.need,
                li.title AS item_title, li.image_key, li.image_status::text AS image_status, li.unit_price_amount, li.open
         FROM wishlists w
         JOIN fr ON fr.id = w.owner_id
         JOIN users o ON o.id = w.owner_id AND o.deleted_at IS NULL AND o.blocked_at IS NULL
         CROSS JOIN LATERAL (
           SELECT count(*) FILTER (WHERE i.created_at > now() - interval '30 days') AS n_new, max(i.created_at) AS at, count(*) AS n,
                  COALESCE(sum({claimed}), 0)::bigint AS got,
                  COALESCE(sum({needed}), 0)::bigint AS need
           FROM wishlist_items i WHERE i.wishlist_id = w.id AND i.deleted_at IS NULL) s
         JOIN LATERAL (
           SELECT i.title, i.image_key, i.image_status, i.unit_price_amount, {open} AS open
           FROM wishlist_items i WHERE i.wishlist_id = w.id AND i.deleted_at IS NULL
           ORDER BY i.created_at DESC, i.id DESC LIMIT 1) li ON true
         WHERE w.deleted_at IS NULL AND w.org_id IS NULL AND w.moderation_status = 'ok' AND w.status = 'active'
           AND (w.closes_at IS NULL OR w.closes_at > now())
           AND (w.visibility::text IN ('public', 'friends')
                OR (w.visibility::text = 'selected' AND EXISTS (SELECT 1 FROM wishlist_allowed_users a WHERE a.wishlist_id = w.id AND a.user_id = $1)))
           AND s.n_new > 0
         ORDER BY s.at DESC LIMIT 30", claimed = crate::wishlists::CLAIMED_SQL, needed = crate::wishlists::NEEDED_SQL, open = crate::wishlists::OPEN_SQL)))
        .bind(u.id).fetch_all(&st.pool).await?;
    let mut events: Vec<(chrono::DateTime<chrono::Utc>, Value)> = lists.into_iter().map(|r| (r.at, json!({
        "kind": "items_added", "at": r.at, "owner": { "id": r.owner_id, "display_name": r.display_name, "handle": r.handle },
        "wishlist": { "slug": r.slug, "title": r.title, "is_new": r.is_new, "event_date": r.event_date, "closes_at": r.closes_at,
                      "item_count": r.n, "completion_pct": crate::wishlists::completion_pct(r.got, r.need) },
        "new_items": r.n_new,
        "item": { "title": r.item_title, "image_url": crate::public::image_url(r.image_key.as_deref(), &r.image_status),
                  "unit_price_amount": r.unit_price_amount, "open": r.open } }))).collect();

    type GiftRow = (
        chrono::DateTime<chrono::Utc>,
        String,
        Option<String>,
        String,
        String,
        String,
        Uuid,
        String,
        Option<String>,
    );
    let gifts: Vec<GiftRow> = sqlx::query_as(
        "SELECT c.delivered_at, i.title, i.image_key, i.image_status::text, w.slug::text, w.title, o.id, o.display_name, o.handle
         FROM claims c
         JOIN wishlist_items i ON i.id = c.item_id
         JOIN wishlists w ON w.id = i.wishlist_id AND w.deleted_at IS NULL AND w.moderation_status = 'ok'
         JOIN users o ON o.id = w.owner_id AND o.deleted_at IS NULL AND o.blocked_at IS NULL
         WHERE c.user_id = $1 AND c.status = 'delivered' AND c.delivered_at > now() - interval '30 days'
         ORDER BY c.delivered_at DESC LIMIT 30")
        .bind(u.id).fetch_all(&st.pool).await?;
    events.extend(
        gifts
            .into_iter()
            .map(|(at, it, ik, is, slug, wt, oid, on, oh)| {
                (
                    at,
                    json!({
        "kind": "delivered", "at": at, "owner": { "id": oid, "display_name": on, "handle": oh },
        "wishlist": { "slug": slug, "title": wt },
        "item": { "title": it, "image_url": crate::public::image_url(ik.as_deref(), &is) } }),
                )
            }),
    );
    events.sort_by_key(|e| std::cmp::Reverse(e.0));
    events.truncate(30);

    let rows: Vec<(Uuid, String, Option<String>, String)> = sqlx::query_as(
        "SELECT o.id, o.display_name, o.handle, o.birthday FROM friendships f
         JOIN users o ON o.id = CASE WHEN f.user_a = $1 THEN f.user_b ELSE f.user_a END AND o.deleted_at IS NULL
         WHERE (f.user_a = $1 OR f.user_b = $1) AND o.birthday IS NOT NULL AND o.birthday_visibility::text IN ('public', 'friends')")
        .bind(u.id).fetch_all(&st.pool).await?;
    let today = (chrono::Utc::now() + chrono::Duration::hours(8)).date_naive(); // 台北日期
    let mut birthdays: Vec<(i64, Value)> = rows
        .into_iter()
        .filter_map(|(id, n, h, b)| {
            let d = days_until(today, &b).filter(|d| *d <= 30)?;
            Some((
                d,
                json!({ "id": id, "display_name": n, "handle": h, "birthday": b, "days": d }),
            ))
        })
        .collect();
    birthdays.sort_by_key(|b| b.0);

    Ok(Json(json!({
        "events": events.into_iter().map(|e| e.1).collect::<Vec<_>>(),
        "birthdays": birthdays.into_iter().map(|b| b.1).collect::<Vec<_>>() })))
}

/// 距離下一個 MM-DD 的天數（今天為 0）；非閏年的 02-29 以 02-28 計
pub(crate) fn days_until(today: chrono::NaiveDate, mmdd: &str) -> Option<i64> {
    use chrono::Datelike;
    let (m, d) = mmdd.split_once('-')?;
    let (m, d): (u32, u32) = (m.parse().ok()?, d.parse().ok()?);
    let on = |y| {
        chrono::NaiveDate::from_ymd_opt(y, m, d).or_else(|| {
            (m == 2 && d == 29)
                .then(|| chrono::NaiveDate::from_ymd_opt(y, 2, 28))
                .flatten()
        })
    };
    let this = on(today.year())?;
    let next = if this >= today {
        this
    } else {
        on(today.year() + 1)?
    };
    Some((next - today).num_days())
}

#[cfg(test)]
mod tests {
    use super::days_until;
    use chrono::NaiveDate;
    #[test]
    fn birthday_countdown() {
        let d = |y, m, dd| NaiveDate::from_ymd_opt(y, m, dd).unwrap();
        assert_eq!(days_until(d(2026, 10, 20), "10-20"), Some(0));
        assert_eq!(days_until(d(2026, 10, 20), "10-24"), Some(4));
        assert_eq!(days_until(d(2026, 12, 30), "01-02"), Some(3));
        assert_eq!(days_until(d(2026, 10, 21), "10-20"), Some(364));
        assert_eq!(days_until(d(2027, 2, 1), "02-29"), Some(27));
        assert_eq!(days_until(d(2028, 2, 1), "02-29"), Some(28));
        assert_eq!(days_until(d(2026, 1, 1), "bad"), None);
        assert_eq!(days_until(d(2027, 2, 28), "02-29"), Some(0)); // 非閏年 2/29 以 2/28 計
        assert_eq!(days_until(d(2027, 3, 1), "02-29"), Some(365));
    }
}
