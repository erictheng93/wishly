//! 個人頁 GET /users/{handle}：依觀看者身分列出可見的清單與捐助。見 0005_social。
use crate::{access, error::AppError, AppState};
use axum::{
    extract::{Path, State},
    http::request::Parts,
    routing::get,
    Json, Router,
};
use chrono::{DateTime, NaiveDate, Utc};
use serde_json::{json, Value};
use uuid::Uuid;

type ProfileWishlist = (
    String,
    String,
    String,
    String,
    String,
    Option<String>,
    String,
    Option<String>,
    Option<NaiveDate>,
    i64,
    i64,
    i64,
);
type Donation = (
    String,
    String,
    Option<i32>,
    Option<i64>,
    String,
    DateTime<Utc>,
    Option<String>,
    String,
    String,
    Option<String>,
);

pub fn routes() -> Router<AppState> {
    Router::new().route("/users/{handle}", get(profile))
}

async fn profile(
    State(st): State<AppState>,
    Path(handle): Path<String>,
    parts: Parts,
) -> Result<Json<Value>, AppError> {
    let viewer = access::viewer(&parts, &st).await?;
    let (uid, name, bio, birthday, bv): (Uuid, String, Option<String>, Option<String>, String) = sqlx::query_as(
        "SELECT id, display_name, bio, birthday, birthday_visibility::text FROM users WHERE handle = $1 AND deleted_at IS NULL AND blocked_at IS NULL",
    )
    .bind(handle.to_lowercase())
    .fetch_optional(&st.pool)
    .await?
    .ok_or(AppError::NotFound)?;
    let is_self = viewer == Some(uid);
    let friend = match viewer {
        Some(v) => access::are_friends(&st.pool, uid, v).await?,
        None => false,
    };
    let relation = match viewer {
        None => "anonymous",
        Some(_) if is_self => "self",
        Some(_) if friend => "friend",
        Some(v) => {
            let (inc, out): (bool, bool) = sqlx::query_as(
                "SELECT EXISTS (SELECT 1 FROM friend_requests WHERE from_user = $1 AND to_user = $2),
                        EXISTS (SELECT 1 FROM friend_requests WHERE from_user = $2 AND to_user = $1)").bind(uid).bind(v).fetch_one(&st.pool).await?;
            if inc {
                "incoming"
            } else if out {
                "outgoing"
            } else {
                "none"
            }
        }
    };

    let birthday = birthday.filter(|_| bv == "public" || is_self || (bv == "friends" && friend));

    // 清單：public 一律；friends 需好友；selected 需在名單；本人另看 friends/selected（private 不列）
    // 完成度與分享頁同規則（眾籌品項達標算 1/1，見 wishlists::units / CLAIMED_SQL）
    let lists: Vec<ProfileWishlist> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT w.slug::text, w.title, w.type::text, w.status::text, w.visibility::text, w.cover_image_key, w.cover_image_status::text, w.cover_preset, w.event_date,
                (SELECT count(*) FROM wishlist_items i WHERE i.wishlist_id = w.id AND i.deleted_at IS NULL),
                (SELECT COALESCE(sum({claimed}), 0) FROM wishlist_items i WHERE i.wishlist_id = w.id AND i.deleted_at IS NULL),
                (SELECT COALESCE(sum(CASE WHEN funding_mode = 'crowdfund' THEN 1 ELSE qty_needed END), 0) FROM wishlist_items i WHERE i.wishlist_id = w.id AND i.deleted_at IS NULL)
         FROM wishlists w
         WHERE w.owner_id = $1 AND w.deleted_at IS NULL AND w.moderation_status = 'ok' AND w.status IN ('active', 'closed')
           AND (w.visibility::text = 'public'
                OR (w.visibility::text IN ('friends', 'selected') AND $3)
                OR (w.visibility::text = 'friends' AND $4)
                OR (w.visibility::text = 'selected' AND EXISTS (SELECT 1 FROM wishlist_allowed_users a WHERE a.wishlist_id = w.id AND a.user_id = $2)))
         ORDER BY w.created_at DESC, w.id DESC", claimed = crate::wishlists::CLAIMED_SQL)))
        .bind(uid).bind(viewer).bind(is_self).bind(friend).fetch_all(&st.pool).await?;
    let wishlists: Vec<Value> = lists
        .into_iter()
        .map(|(slug, title, ty, status, vis, ck, cs, cp, ev, n, got, need)| {
            json!({
        "slug": slug, "title": title, "type": ty, "status": status, "visibility": vis,
        "cover_image_url": crate::public::image_url(ck.as_deref(), &cs), "cover_preset": cp, "event_date": ev,
        "item_count": n, "completion_pct": crate::wishlists::completion_pct(got, need) })
        })
        .collect();

    // 捐助 = 捐助 ∪ 點數認捐。可見性（$3=本人、$4=與捐助者為好友，與 access::claim_visible 同語意）：
    //   捐助看 c.visibility、認捐看 k.visibility，且兩者的匿名紀錄都只給本人看（不能從個人頁反推匿名者）。
    // 清單條件兩者共用：對觀看者可存取（public；friends/selected 依觀看者或擁有者）、排除驚喜鎖定。
    let rows: Vec<Donation> = sqlx::query_as(
        "WITH d AS (
           SELECT 'claim' AS kind, c.id, c.item_id, c.qty, NULL::bigint AS points, c.status::text AS status, c.created_at
           FROM claims c
           WHERE c.user_id = $1 AND c.status IN ('reserved', 'purchased', 'delivered')
             AND ($3 OR (NOT c.is_anonymous AND (c.visibility::text = 'public' OR (c.visibility::text = 'friends' AND $4))))
           UNION ALL
           SELECT 'pledge', k.id, k.item_id, NULL, k.points, k.status::text, k.created_at
           FROM contributions k
           WHERE k.user_id = $1 AND k.status IN ('pledged', 'captured')
             AND ($3 OR (NOT k.is_anonymous AND (k.visibility::text = 'public' OR (k.visibility::text = 'friends' AND $4)))))
         SELECT d.kind, i.title, d.qty, d.points, d.status, d.created_at,
                w.slug::text, w.title, o.display_name, o.handle
         FROM d
         JOIN wishlist_items i ON i.id = d.item_id
         JOIN wishlists w ON w.id = i.wishlist_id
         JOIN users o ON o.id = w.owner_id AND o.blocked_at IS NULL
         WHERE w.deleted_at IS NULL AND w.moderation_status = 'ok' AND w.status IN ('active', 'closed')
           AND NOT surprise_locked(w.surprise_mode, w.event_date)
           AND (w.visibility::text = 'public'
                OR w.owner_id = $2
                OR (w.visibility::text = 'friends' AND $2::uuid IS NOT NULL
                    AND EXISTS (SELECT 1 FROM friendships f WHERE f.user_a = LEAST($2::uuid, w.owner_id) AND f.user_b = GREATEST($2::uuid, w.owner_id)))
                OR (w.visibility::text = 'selected' AND EXISTS (SELECT 1 FROM wishlist_allowed_users a WHERE a.wishlist_id = w.id AND a.user_id = $2)))
         ORDER BY d.created_at DESC, d.id DESC LIMIT 50")
        .bind(uid).bind(viewer).bind(is_self).bind(friend).fetch_all(&st.pool).await?;
    let donations: Vec<Value> = rows.into_iter().map(|(kind, it, qty, points, status, at, slug, wt, on, oh)| json!({
        "kind": kind, "item_title": it, "qty": qty, "points": points, "status": status, "created_at": at,
        "wishlist": { "slug": slug, "title": wt, "owner": { "display_name": on, "handle": oh } } })).collect();

    Ok(Json(
        json!({ "user": { "id": uid, "handle": handle.to_lowercase(), "display_name": name, "bio": bio, "birthday": birthday }, "relation": relation,
                    "wishlists": wishlists, "donations": donations }),
    ))
}
