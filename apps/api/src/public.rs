//! GET /public/wishlists/{slug}（契約第 5 章；訪客視角，可被 CDN 快取，只出 public 內容）
//! GET /restricted/wishlists/{slug}（好友 / 指定 / 擁有者，依 ws_session，永遠 no-store）
use crate::{error::AppError, AppState};
use axum::{
    extract::{Path, State},
    http::{header, request::Parts, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use chrono::{DateTime, NaiveDate, Utc};
use serde_json::{json, Value};
use sqlx::FromRow;
use std::collections::HashMap;
use uuid::Uuid;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/public/wishlists", get(explore))
        .route("/public/wishlists/{slug}", get(wishlist))
        .route("/restricted/wishlists/{slug}", get(restricted))
}

const EXPLORE_PAGE: i64 = 24;

#[derive(serde::Deserialize)]
struct ExploreQ {
    page: Option<i64>,
    r#type: Option<String>,
}

type ExploreRow = (String, String, String, Option<String>, String, Option<NaiveDate>, String, Option<String>, i64, i64, i64);

/// GET /public/wishlists?page=&type=：探索頁，列出所有人的公開清單（訪客可看；不讀 cookie，可被 CDN 快取）。
/// 條件與 `servable` 一致再加：有品項、未結束、未驚喜鎖定、擁有者正常。
/// ponytail: OFFSET 分頁（page 上限 100），清單到數萬筆再改 keyset；清單被收緊時探索頁最多殘留約一分鐘的快取卡片，點進去仍是 403/404。
async fn explore(
    State(st): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<ExploreQ>,
) -> Result<Response, AppError> {
    let page = q.page.unwrap_or(1).clamp(1, 100);
    let ty = match q.r#type.as_deref() {
        None | Some("") => None,
        Some(t @ ("registry" | "relief")) => Some(t),
        _ => return Err(AppError::invalid("/type", "ENUM", "type 必須是 registry / relief")),
    };
    let rows: Vec<ExploreRow> = sqlx::query_as(&format!(
        "SELECT w.slug::text, w.title, w.type::text, w.cover_image_key, w.cover_image_status::text, w.event_date,
                o.display_name, o.handle,
                (SELECT count(*) FROM wishlist_items i WHERE i.wishlist_id = w.id AND i.deleted_at IS NULL),
                (SELECT COALESCE(sum({claimed}), 0)::bigint FROM wishlist_items i WHERE i.wishlist_id = w.id AND i.deleted_at IS NULL),
                (SELECT COALESCE(sum(CASE WHEN funding_mode = 'crowdfund' THEN 1 ELSE qty_needed END), 0)::bigint FROM wishlist_items i WHERE i.wishlist_id = w.id AND i.deleted_at IS NULL)
         FROM wishlists w JOIN users o ON o.id = w.owner_id
         WHERE w.visibility = 'public' AND w.status = 'active' AND w.deleted_at IS NULL AND w.moderation_status = 'ok'
           AND o.deleted_at IS NULL AND o.blocked_at IS NULL
           AND (w.closes_at IS NULL OR w.closes_at > now())
           AND w.slug::text NOT LIKE 'demo%'   -- 0018 範例清單不上探索頁
           AND NOT (w.surprise_mode AND (w.event_date IS NULL OR now() < (w.event_date::timestamp AT TIME ZONE 'Asia/Taipei')))
           AND ($1::text IS NULL OR w.type::text = $1)
           AND EXISTS (SELECT 1 FROM wishlist_items i WHERE i.wishlist_id = w.id AND i.deleted_at IS NULL)
         ORDER BY w.created_at DESC, w.id DESC LIMIT $2 OFFSET $3", claimed = crate::wishlists::CLAIMED_SQL))
        .bind(ty).bind(EXPLORE_PAGE + 1).bind((page - 1) * EXPLORE_PAGE).fetch_all(&st.pool).await?;
    let more = rows.len() as i64 > EXPLORE_PAGE;
    let wishlists: Vec<Value> = rows.into_iter().take(EXPLORE_PAGE as usize).map(|(slug, title, ty, ck, cs, ev, on, oh, n, got, need)| json!({
        "slug": slug, "title": title, "type": ty, "cover_image_url": image_url(ck.as_deref(), &cs), "event_date": ev,
        "owner": { "display_name": on, "handle": oh }, "item_count": n, "completion_pct": crate::wishlists::completion_pct(got, need) })).collect();
    Ok((
        [(header::CACHE_CONTROL, "public, s-maxage=10, stale-while-revalidate=60")],
        Json(json!({ "wishlists": wishlists, "next_page": more.then_some(page + 1) })),
    ).into_response())
}

pub fn image_url(key: Option<&str>, status: &str) -> Option<String> {
    key.filter(|_| status == "ready")
        .map(|k| crate::uploads::S3::get().public_url(k))
}

#[derive(FromRow)]
struct W {
    id: Uuid,
    slug: String,
    r#type: String,
    status: String,
    visibility: String,
    moderation: String,
    title: String,
    description: Option<String>,
    cover_image_key: Option<String>,
    cover_image_status: String,
    event_date: Option<NaiveDate>,
    closes_at: Option<DateTime<Utc>>,
    show_claimer_names: bool,
    surprise_mode: bool,
    owner_id: Uuid,
    owner_name: String,
    locked: bool,
    updated_at: DateTime<Utc>,
}

#[derive(FromRow)]
struct It {
    id: Uuid,
    title: String,
    brand: Option<String>,
    spec: Option<String>,
    image_key: Option<String>,
    image_status: String,
    product_url: Option<String>,
    unit_price_amount: Option<i64>,
    priority: String,
    funding_mode: String,
    qty_needed: i32,
    qty_claimed: i32,
    target_points: Option<i64>,
    pledged_points: i64,
    funding_status: Option<String>,
    display_status: Option<String>,
    funding_deadline: Option<DateTime<Utc>>,
}

/// 可被 CDN 快取的路徑：**不讀 cookie**，只會回 public 清單內容；
/// 受限清單（friends / selected / private）在這裡一律以匿名身分判斷 → 只會得到不含清單內容的 403 / 404。
/// 這樣即使 Cloudflare 被設成強制快取這個路徑，也只可能快取到公開資料或空的門檻回應。
async fn wishlist(
    State(st): State<AppState>,
    Path(slug): Path<String>,
    parts: Parts,
) -> Result<Response, AppError> {
    render(&st, &slug, None, &parts).await
}

/// 受限清單內容：依 ws_session 判斷，任何清單都回 private, no-store（不走 CDN 快取）。
async fn restricted(
    State(st): State<AppState>,
    Path(slug): Path<String>,
    parts: Parts,
) -> Result<Response, AppError> {
    let viewer = crate::access::viewer(&parts, &st).await?;
    render(&st, &slug, Some(viewer), &parts).await
}

/// who = None：公開（可快取）路徑，匿名判斷；Some(登入者)：受限路徑。
async fn render(
    st: &AppState,
    slug: &str,
    who: Option<Option<Uuid>>,
    parts: &Parts,
) -> Result<Response, AppError> {
    if !crate::validate::slug_ok(slug) {
        return Err(AppError::NotFound);
    }
    let w: W = sqlx::query_as(
        "SELECT w.id, w.slug::text AS slug, w.type::text AS type, w.status::text AS status, w.visibility::text AS visibility,
                w.moderation_status::text AS moderation, w.title, w.description, w.cover_image_key,
                w.cover_image_status::text AS cover_image_status, w.event_date, w.closes_at, w.show_claimer_names, w.surprise_mode,
                w.owner_id, u.display_name AS owner_name,
                (w.surprise_mode AND (w.event_date IS NULL OR now() < (w.event_date::timestamp AT TIME ZONE 'Asia/Taipei'))) AS locked,
                GREATEST(w.updated_at, COALESCE((SELECT max(updated_at) FROM wishlist_items WHERE wishlist_id = w.id), w.updated_at)) AS updated_at
         FROM wishlists w JOIN users u ON u.id = w.owner_id
         WHERE w.slug = $1 AND w.deleted_at IS NULL")
        .bind(slug).fetch_optional(&st.pool).await?.ok_or(AppError::NotFound)?;
    if w.status == "draft" || w.status == "archived" {
        return Err(AppError::NotFound);
    }
    let viewer = who.flatten();
    crate::access::check_wishlist(
        &st.pool,
        &crate::access::ListAccess {
            wishlist_id: w.id,
            owner_id: w.owner_id,
            visibility: &w.visibility,
        },
        viewer,
    )
    .await?;
    if w.moderation == "hidden" {
        return Err(AppError::WishlistRemoved);
    }

    // 只有公開路徑上的 public 可被共用快取；受限路徑一律 no-store、不出 ETag（依觀看者而異）
    let restricted = who.is_some() || w.visibility != "public";
    let etag = format!(
        "\"w-{}-{}{}\"",
        w.slug,
        w.updated_at.timestamp_millis(),
        if w.locked { "-l" } else { "" }
    );
    let cache = [(
        header::CACHE_CONTROL,
        if restricted {
            "private, no-store"
        } else {
            "public, s-maxage=10, stale-while-revalidate=60"
        }
        .to_string(),
    )];
    if !restricted
        && parts
            .headers
            .get(header::IF_NONE_MATCH)
            .and_then(|v| v.to_str().ok())
            == Some(etag.as_str())
    {
        return Ok((StatusCode::NOT_MODIFIED, cache, [(header::ETAG, etag)]).into_response());
    }

    let items: Vec<It> = sqlx::query_as(
        &format!("SELECT i.id, i.title, i.brand, i.spec, i.image_key, i.image_status::text AS image_status, i.product_url, i.unit_price_amount,
                i.priority::text AS priority, i.funding_mode::text AS funding_mode, i.qty_needed, i.qty_claimed,
                i.target_points, i.pledged_points, i.funding_status::text AS funding_status, ({}) AS display_status, i.funding_deadline
         FROM wishlist_items i LEFT JOIN purchase_orders po ON po.item_id = i.id
         WHERE i.wishlist_id = $1 AND i.deleted_at IS NULL ORDER BY i.sort_order, i.id", crate::points::DISPLAY_STATUS_SQL))
        .bind(w.id).fetch_all(&st.pool).await?;

    // 驚喜鎖定期間無論 show_claimer_names 為何一律不輸出（伺服器端強制）
    let visible = w.show_claimer_names && !w.locked;
    let mut claimers: HashMap<Uuid, Vec<Value>> = HashMap::new();
    if visible {
        let rows: Vec<(Uuid, String, i64)> = sqlx::query_as(
            // 登入者的捐助非 public → 對外顯示「匿名」（此回應可被共用快取，不做逐觀看者的好友判斷）
            "SELECT c.item_id, CASE WHEN c.visibility IS NULL OR c.visibility = 'public' THEN c.claimer_name ELSE '匿名' END AS n, sum(c.qty)::bigint
             FROM claims c JOIN wishlist_items i ON i.id = c.item_id
             WHERE i.wishlist_id = $1 AND i.deleted_at IS NULL AND c.status IN ('reserved','purchased','delivered')
             GROUP BY c.item_id, n ORDER BY min(c.created_at)")
            .bind(w.id).fetch_all(&st.pool).await?;
        for (item, name, qty) in rows {
            claimers
                .entry(item)
                .or_default()
                .push(json!({ "display_name": name, "qty": qty }));
        }
    }

    // 眾籌捐贈者：同一位（同 user 且非匿名）合併成一筆；匿名者各自一筆、一律「匿名朋友」。只算 pledged / captured。
    // 與 claimers 同一道閘：show_claimer_names 且未驚喜鎖定才輸出。
    let mut contributors: HashMap<Uuid, Vec<Value>> = HashMap::new();
    if visible {
        let rows: Vec<(Uuid, Uuid, bool, String, i64)> = sqlx::query_as(
            "SELECT c.item_id, c.user_id, c.is_anonymous, c.donor_name, c.points FROM contributions c JOIN wishlist_items i ON i.id = c.item_id
             WHERE i.wishlist_id = $1 AND i.deleted_at IS NULL AND c.status IN ('pledged', 'captured') ORDER BY c.created_at, c.id")
            .bind(w.id).fetch_all(&st.pool).await?;
        let mut merged: HashMap<(Uuid, Uuid), usize> = HashMap::new(); // (item, user) -> 該品項 contributors 的索引（僅非匿名）
        for (item, user, anon, name, points) in rows {
            let list = contributors.entry(item).or_default();
            if anon {
                list.push(json!({ "display_name": "匿名朋友", "points": points }));
                continue;
            }
            match merged.get(&(item, user)) {
                Some(&k) => {
                    list[k]["points"] = json!(list[k]["points"].as_i64().unwrap_or(0) + points)
                }
                None => {
                    merged.insert((item, user), list.len());
                    list.push(json!({ "display_name": name, "points": points }));
                }
            }
        }
    }

    // 完成度：數量型 = qty_claimed/qty_needed；眾籌型達標（funded / fulfilled）算 1/1（見 wishlists::units）
    let (mut total, mut got, mut fulfilled) = (0i64, 0i64, 0usize);
    for i in &items {
        let (c, n) = crate::wishlists::units(
            &i.funding_mode,
            i.qty_needed,
            i.qty_claimed,
            i.funding_status.as_deref(),
        );
        total += n;
        got += c;
        if c >= n {
            fulfilled += 1;
        }
    }
    let pct = crate::wishlists::completion_pct;
    let item_json: Vec<Value> = items.iter().map(|i| {
        let cf = i.funding_mode == "crowdfund";
        let funded = matches!(i.funding_status.as_deref(), Some("funded" | "fulfilled"));
        let mut v = json!({
            "id": i.id, "title": i.title, "brand": i.brand, "spec": i.spec,
            "image_url": image_url(i.image_key.as_deref(), &i.image_status),
            "product_url": i.product_url, "unit_price_amount": i.unit_price_amount, "priority": i.priority,
            "funding_mode": i.funding_mode, "qty_needed": i.qty_needed, "qty_claimed": i.qty_claimed,
            "qty_remaining": i.qty_needed - i.qty_claimed, "is_fully_claimed": if cf { funded } else { i.qty_claimed >= i.qty_needed },
            "target_points": i.target_points, "pledged_points": cf.then_some(i.pledged_points),
            "remaining_points": i.target_points.filter(|_| cf).map(|t| (t - i.pledged_points).max(0)),
            "funding_status": i.funding_status, "display_status": i.display_status, "funding_deadline": i.funding_deadline,
            "progress_percent": if cf { pct(i.pledged_points, i.target_points.unwrap_or(0)) } else { pct(i.qty_claimed as i64, i.qty_needed as i64) },
        });
        if visible {
            v["claimers"] = json!(claimers.remove(&i.id).unwrap_or_default());
            if cf { v["contributors"] = json!(contributors.remove(&i.id).unwrap_or_default()); }
        }
        v
    }).collect();

    let body = json!({
        "slug": w.slug, "visibility": w.visibility, "type": w.r#type, "status": w.status, "title": w.title, "description": w.description,
        "cover_image_url": image_url(w.cover_image_key.as_deref(), &w.cover_image_status),
        "event_date": w.event_date, "closes_at": w.closes_at, "id": w.id,
        "owner": { "id": w.owner_id, "display_name": w.owner_name },
        "surprise_mode": w.surprise_mode, "claimers_visible": visible,
        "completion": { "item_count": items.len(), "fulfilled_count": fulfilled, "completion_pct": pct(got, total) },
        "items": item_json, "updated_at": w.updated_at,
    });
    if restricted {
        return Ok((cache, Json(body)).into_response());
    }
    Ok((cache, [(header::ETAG, etag)], Json(body)).into_response())
}
