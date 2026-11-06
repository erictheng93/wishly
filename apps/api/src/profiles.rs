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
    // 沒設 handle 的人用 user id 進個人頁；handle 最長 30 字且不含 '-'，不會與 UUID 撞。
    // 用 id 只給登入者看：handle 是本人選擇的對外網址，沒設的人不該有能匿名轉傳的個人頁
    let by_id = Uuid::try_parse(&handle).ok();
    if by_id.is_some() && viewer.is_none() {
        return Err(AppError::NotFound);
    }
    #[allow(clippy::type_complexity)]
    let (uid, handle, name, bio, birthday, bv, avatar_key): (Uuid, Option<String>, String, Option<String>, Option<String>, String, Option<String>) = sqlx::query_as(
        "SELECT id, handle, display_name, bio, birthday, birthday_visibility::text, avatar_key FROM users
          WHERE (id = $1 OR ($1 IS NULL AND handle = $2)) AND deleted_at IS NULL AND blocked_at IS NULL",
    )
    .bind(by_id)
    .bind(handle.to_lowercase())
    .fetch_optional(&st.pool)
    .await?
    .ok_or(AppError::NotFound)?;
    let is_self = viewer == Some(uid);
    // 封鎖：對方封鎖了我 → 404（不洩漏）；我封鎖對方 → 只回基本資料，relation = "blocked"
    let mut i_blocked = false;
    if let (Some(v), false) = (viewer, is_self) {
        let (they, me): (bool, bool) = sqlx::query_as(
            "SELECT EXISTS (SELECT 1 FROM user_blocks WHERE blocker_id = $1 AND blocked_id = $2),
                    EXISTS (SELECT 1 FROM user_blocks WHERE blocker_id = $2 AND blocked_id = $1)",
        )
        .bind(uid)
        .bind(v)
        .fetch_one(&st.pool)
        .await?;
        if they {
            return Err(AppError::NotFound);
        }
        i_blocked = me;
    }
    if i_blocked {
        return Ok(Json(
            json!({ "user": { "id": uid, "handle": handle, "display_name": name, "bio": bio, "birthday": null,
                              "avatar_url": crate::uploads::avatar_url(avatar_key.as_deref()) },
                    "relation": "blocked", "request_id": null, "mutual": null,
                    "wishlists": [], "donations": [] }),
        ));
    }
    let friend = match viewer {
        Some(v) => access::are_friends(&st.pool, uid, v).await?,
        None => false,
    };
    let mut request_id: Option<Uuid> = None;
    let relation = match viewer {
        None => "anonymous",
        Some(_) if is_self => "self",
        Some(_) if friend => "friend",
        Some(v) => {
            // 兩個方向各自唯一；incoming（對方寄給我）優先
            let (inc, out): (Option<Uuid>, Option<Uuid>) = sqlx::query_as(
                "SELECT (SELECT id FROM friend_requests WHERE from_user = $1 AND to_user = $2),
                        (SELECT id FROM friend_requests WHERE from_user = $2 AND to_user = $1)",
            )
            .bind(uid)
            .bind(v)
            .fetch_one(&st.pool)
            .await?;
            request_id = inc.or(out);
            if inc.is_some() {
                "incoming"
            } else if out.is_some() {
                "outgoing"
            } else {
                "none"
            }
        }
    };

    // 共同好友：同時是觀看者與本人好友者（匿名與本人不顯示）；排除已刪除 / 停權。
    // 刻意比照 Facebook：任何登入者（不必是好友）都看得到，產品決策，不是外洩
    let mutual = match viewer {
        Some(v) if !is_self => {
            // 從兩人的好友（PK user_a / friendships_b_idx）取交集，不掃 users
            #[allow(clippy::type_complexity)]
            let rows: Vec<(Uuid, Option<String>, String, Option<String>, i64)> = sqlx::query_as(
                "WITH a AS (SELECT CASE WHEN user_a = $1 THEN user_b ELSE user_a END AS id FROM friendships WHERE user_a = $1 OR user_b = $1),
                      b AS (SELECT CASE WHEN user_a = $2 THEN user_b ELSE user_a END AS id FROM friendships WHERE user_a = $2 OR user_b = $2)
                 SELECT u.id, u.handle, u.display_name, u.avatar_key, count(*) OVER ()
                   FROM a JOIN b USING (id) JOIN users u ON u.id = a.id
                  WHERE u.deleted_at IS NULL AND u.blocked_at IS NULL
                    AND NOT EXISTS (SELECT 1 FROM user_blocks k WHERE (k.blocker_id = u.id AND k.blocked_id IN ($1, $2)) OR (k.blocked_id = u.id AND k.blocker_id IN ($1, $2)))
                  ORDER BY u.display_name, u.id LIMIT 3")
                .bind(uid).bind(v).fetch_all(&st.pool).await?;
            let count = rows.first().map_or(0, |r| r.4);
            let users: Vec<Value> = rows.iter().map(|(id, h, n, a, _)| json!({
                "id": id, "handle": h, "display_name": n, "avatar_url": crate::uploads::avatar_url(a.as_deref()) })).collect();
            json!({ "count": count, "users": users })
        }
        _ => Value::Null,
    };

    let birthday = birthday.filter(|_| bv == "public" || is_self || (bv == "friends" && friend));

    // 清單：public 一律；friends 需好友；selected 需在名單；本人另看 friends/selected（private 不列）
    // 完成度與分享頁同規則（眾籌品項達標算 1/1，見 wishlists::units / CLAIMED_SQL）
    let lists: Vec<ProfileWishlist> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT w.slug::text, w.title, w.type::text, w.status::text, w.visibility::text, w.cover_image_key, w.cover_image_status::text, w.cover_preset, w.event_date,
                (SELECT count(*) FROM wishlist_items i WHERE i.wishlist_id = w.id AND i.deleted_at IS NULL),
                (SELECT COALESCE(sum({claimed}), 0) FROM wishlist_items i WHERE i.wishlist_id = w.id AND i.deleted_at IS NULL),
                (SELECT COALESCE(sum({needed}), 0) FROM wishlist_items i WHERE i.wishlist_id = w.id AND i.deleted_at IS NULL)
         FROM wishlists w
         WHERE w.owner_id = $1 AND w.org_id IS NULL AND w.deleted_at IS NULL AND w.moderation_status = 'ok' AND w.status IN ('active', 'closed')
           AND (w.visibility::text = 'public'
                OR (w.visibility::text IN ('friends', 'selected') AND $3)
                OR (w.visibility::text = 'friends' AND $4)
                OR (w.visibility::text = 'selected' AND EXISTS (SELECT 1 FROM close_friends c WHERE c.owner_id = w.owner_id AND c.friend_id = $2)))
         ORDER BY w.created_at DESC, w.id DESC", claimed = crate::wishlists::CLAIMED_SQL, needed = crate::wishlists::NEEDED_SQL)))
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
                OR (w.visibility::text = 'selected' AND EXISTS (SELECT 1 FROM close_friends c WHERE c.owner_id = w.owner_id AND c.friend_id = $2)))
         ORDER BY d.created_at DESC, d.id DESC LIMIT 50")
        .bind(uid).bind(viewer).bind(is_self).bind(friend).fetch_all(&st.pool).await?;
    let donations: Vec<Value> = rows.into_iter().map(|(kind, it, qty, points, status, at, slug, wt, on, oh)| json!({
        "kind": kind, "item_title": it, "qty": qty, "points": points, "status": status, "created_at": at,
        "wishlist": { "slug": slug, "title": wt, "owner": { "display_name": on, "handle": oh } } })).collect();

    let links = crate::user_links::visible(&st.pool, uid).await?;
    Ok(Json(
        json!({ "user": { "id": uid, "handle": handle, "display_name": name, "bio": bio, "birthday": birthday,
                              "avatar_url": crate::uploads::avatar_url(avatar_key.as_deref()), "links": links },
                    "relation": relation, "request_id": request_id, "mutual": mutual,
                    "wishlists": wishlists, "donations": donations }),
    ))
}
