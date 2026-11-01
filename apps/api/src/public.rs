//! GET /public/wishlists/{slug}（契約第 5 章；訪客視角，可被 CDN 快取，只出 public 內容）
//! GET /public/status（前端維護橫幅用；只回唯讀旗標，可被 CDN 快取 30 秒）
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
        .route("/public/status", get(status))
        .route("/public/wishlists", get(explore))
        .route("/public/wishlists/{slug}", get(wishlist))
        .route("/restricted/wishlists/{slug}", get(restricted))
        .route("/public/relief-events", get(relief::events))
        .route("/public/relief-events/{slug}", get(relief::event))
        .route("/public/orgs/{slug}", get(relief::org))
}

/// 唯讀維護中就讓前端先掛橫幅，訪客不必填完表單才被 503 擋下。
async fn status(State(st): State<AppState>) -> Result<Response, AppError> {
    let v: Option<Value> =
        sqlx::query_scalar("SELECT value FROM system_flags WHERE key = 'read_only'")
            .fetch_optional(&st.pool)
            .await?;
    let read_only = v.and_then(|v| v.as_bool()) == Some(true);
    let cfg = crate::config::get();
    // 儲值可用的付款方式（設定是靜態的，可隨 CDN 快取）；未設綠界憑證則為空
    let topup_methods: Vec<&str> = cfg.ecpay.as_ref().map_or(vec![], |e| {
        let mut m = vec![];
        if e.card_enabled {
            m.push("card");
        }
        m.extend(["cvs_code", "cvs_barcode", "atm"]);
        m
    });
    Ok((
        [(header::CACHE_CONTROL, "public, max-age=30")],
        Json(json!({ "read_only": read_only, "orgs_enabled": cfg.orgs_enabled, "topup_methods": topup_methods })),
    )
        .into_response())
}

const EXPLORE_PAGE: i64 = 24;

#[derive(serde::Deserialize)]
struct ExploreQ {
    page: Option<i64>,
    r#type: Option<String>,
}

type ExploreRow = (
    String,
    String,
    String,
    Option<String>,
    String,
    Option<String>,
    Option<NaiveDate>,
    String,
    Option<String>,
    i64,
    i64,
    i64,
);

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
        _ => {
            return Err(AppError::invalid(
                "/type",
                "ENUM",
                "type 必須是 registry / relief",
            ))
        }
    };
    let rows: Vec<ExploreRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT w.slug::text, w.title, w.type::text, w.cover_image_key, w.cover_image_status::text, w.cover_preset, w.event_date,
                o.display_name, o.handle,
                (SELECT count(*) FROM wishlist_items i WHERE i.wishlist_id = w.id AND i.deleted_at IS NULL),
                (SELECT COALESCE(sum({claimed}), 0)::bigint FROM wishlist_items i WHERE i.wishlist_id = w.id AND i.deleted_at IS NULL),
                (SELECT COALESCE(sum({needed}), 0)::bigint FROM wishlist_items i WHERE i.wishlist_id = w.id AND i.deleted_at IS NULL)
         FROM wishlists w JOIN users o ON o.id = w.owner_id
         WHERE w.visibility = 'public' AND w.status = 'active' AND w.deleted_at IS NULL AND w.moderation_status = 'ok'
           AND o.deleted_at IS NULL AND o.blocked_at IS NULL
           AND (w.closes_at IS NULL OR w.closes_at > now())
           AND w.slug::text NOT LIKE 'demo%'   -- 範例清單（db/seed/demo.sql）不上探索頁
           AND w.type <> 'relief'   -- 救災據點只走 /public/relief-events
           AND NOT surprise_locked(w.surprise_mode, w.event_date)
           AND ($1::text IS NULL OR w.type::text = $1)
           AND EXISTS (SELECT 1 FROM wishlist_items i WHERE i.wishlist_id = w.id AND i.deleted_at IS NULL)
         ORDER BY w.created_at DESC, w.id DESC LIMIT $2 OFFSET $3", claimed = crate::wishlists::CLAIMED_SQL, needed = crate::wishlists::NEEDED_SQL)))
        .bind(ty).bind(EXPLORE_PAGE + 1).bind((page - 1) * EXPLORE_PAGE).fetch_all(&st.pool).await?;
    let more = rows.len() as i64 > EXPLORE_PAGE;
    let wishlists: Vec<Value> = rows.into_iter().take(EXPLORE_PAGE as usize).map(|(slug, title, ty, ck, cs, cp, ev, on, oh, n, got, need)| json!({
        "slug": slug, "title": title, "type": ty, "cover_image_url": image_url(ck.as_deref(), &cs), "cover_preset": cp, "event_date": ev,
        "owner": { "display_name": on, "handle": oh }, "item_count": n, "completion_pct": crate::wishlists::completion_pct(got, need) })).collect();
    Ok((
        [(
            header::CACHE_CONTROL,
            "public, s-maxage=10, stale-while-revalidate=60",
        )],
        Json(json!({ "wishlists": wishlists, "next_page": more.then_some(page + 1) })),
    )
        .into_response())
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
    cover_preset: Option<String>,
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
    category: Option<String>,
    urgency: Option<String>,
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
                w.cover_image_status::text AS cover_image_status, w.cover_preset, w.event_date, w.closes_at, w.show_claimer_names, w.surprise_mode,
                w.owner_id, u.display_name AS owner_name,
                surprise_locked(w.surprise_mode, w.event_date) AS locked,
                GREATEST(w.updated_at, COALESCE((SELECT max(updated_at) FROM wishlist_items WHERE wishlist_id = w.id), w.updated_at)) AS updated_at
         FROM wishlists w JOIN users u ON u.id = w.owner_id
         WHERE w.slug = $1 AND w.deleted_at IS NULL AND u.blocked_at IS NULL")
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
        sqlx::AssertSqlSafe(format!("SELECT i.id, i.title, i.brand, i.spec, i.image_key, i.image_status::text AS image_status, i.product_url, i.unit_price_amount,
                i.priority::text AS priority, i.funding_mode::text AS funding_mode, i.qty_needed, i.qty_claimed,
                i.target_points, i.pledged_points, i.funding_status::text AS funding_status, ({}) AS display_status, i.funding_deadline, i.category, i.urgency
         FROM wishlist_items i LEFT JOIN purchase_orders po ON po.item_id = i.id
         WHERE i.wishlist_id = $1 AND i.deleted_at IS NULL ORDER BY i.sort_order, i.id", crate::points::DISPLAY_STATUS_SQL)))
        .bind(w.id).fetch_all(&st.pool).await?;

    // 驚喜鎖定期間無論 show_claimer_names 為何一律不輸出（伺服器端強制）
    let visible = w.show_claimer_names && !w.locked;
    let mut claimers: HashMap<Uuid, Vec<Value>> = HashMap::new();
    if visible {
        let rows: Vec<(Uuid, String, bool, i64)> = sqlx::query_as(
            // 匿名或登入者的捐助非 public → 對外顯示「匿名」（此回應可被共用快取，不做逐觀看者的好友判斷）
            "SELECT c.item_id, CASE WHEN NOT c.is_anonymous AND (c.visibility IS NULL OR c.visibility = 'public') THEN c.claimer_name ELSE '匿名' END AS n,
                    (c.is_anonymous OR NOT (c.visibility IS NULL OR c.visibility = 'public')) AS anon, sum(c.qty)::bigint
             FROM claims c JOIN wishlist_items i ON i.id = c.item_id
             WHERE i.wishlist_id = $1 AND i.deleted_at IS NULL AND c.status IN ('reserved','purchased','delivered')
             GROUP BY c.item_id, n, anon ORDER BY min(c.created_at)")
            .bind(w.id).fetch_all(&st.pool).await?;
        for (item, name, anon, qty) in rows {
            claimers
                .entry(item)
                .or_default()
                .push(json!({ "display_name": name, "qty": qty, "anonymous": anon }));
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
                list.push(
                    json!({ "display_name": "匿名朋友", "points": points, "anonymous": true }),
                );
                continue;
            }
            match merged.get(&(item, user)) {
                Some(&k) => {
                    list[k]["points"] = json!(list[k]["points"].as_i64().unwrap_or(0) + points)
                }
                None => {
                    merged.insert((item, user), list.len());
                    list.push(
                        json!({ "display_name": name, "points": points, "anonymous": false }),
                    );
                }
            }
        }
    }

    // 完成度：數量型 = qty_claimed/qty_needed；眾籌型達標（funded / fulfilled）算 1/1（見 wishlists::units）
    let (mut total, mut got, mut fulfilled, mut counted) = (0i64, 0i64, 0usize, 0usize);
    for i in &items {
        let (c, n) = crate::wishlists::units(
            &i.funding_mode,
            i.qty_needed,
            i.qty_claimed,
            i.funding_status.as_deref(),
        );
        if n == 0 {
            continue; // 支持品項不計入
        }
        counted += 1;
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
            "category": i.category, "urgency": i.urgency,
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

    let mut body = json!({
        "slug": w.slug, "visibility": w.visibility, "type": w.r#type, "status": w.status, "title": w.title, "description": w.description,
        "cover_image_url": image_url(w.cover_image_key.as_deref(), &w.cover_image_status),
        "cover_preset": w.cover_preset,
        "event_date": w.event_date, "closes_at": w.closes_at, "id": w.id,
        // 救災據點是組織的，不曝露建立者個人資料（改顯示 org）
        "owner": if w.r#type == "relief" { Value::Null } else { json!({ "id": w.owner_id, "display_name": w.owner_name }) },
        "surprise_mode": w.surprise_mode, "surprise_locked": w.locked, "claimers_visible": visible,
        "completion": { "item_count": counted, "fulfilled_count": fulfilled, "completion_pct": pct(got, total) },
        "items": item_json, "updated_at": w.updated_at,
    });
    if w.r#type == "relief" {
        relief::extend_site(&st.pool, w.id, &mut body).await?;
    }
    if restricted {
        return Ok((cache, Json(body)).into_response());
    }
    Ok((cache, [(header::ETAG, etag)], Json(body)).into_response())
}

// ---------- 救災：事件 / 組織公開頁（皆不讀 cookie，可被 CDN 快取） ----------
mod relief {
    use super::*;
    use axum::extract::Query;

    const CACHE: [(header::HeaderName, &str); 1] = [(
        header::CACHE_CONTROL,
        "public, s-maxage=10, stale-while-revalidate=60",
    )];
    const EVENTS_PAGE: i64 = 20;
    /// 公開可見的據點：上線、public、據點狀態 active、未下架、許可未過期（不等每小時 tick 暫停）
    const LIVE_SITE: &str = "w.type = 'relief' AND w.status = 'active' AND w.visibility = 'public' AND w.site_status = 'active'
        AND w.deleted_at IS NULL AND w.moderation_status = 'ok'
        AND EXISTS (SELECT 1 FROM organizations vo WHERE vo.id = w.org_id AND vo.verification_status = 'verified')
        AND NOT EXISTS (SELECT 1 FROM org_permits vp WHERE vp.id = w.permit_id AND (now() AT TIME ZONE 'Asia/Taipei')::date > vp.valid_to)";

    /// relief 清單頁多帶組織 / 事件 / 據點資訊
    pub async fn extend_site(
        pool: &sqlx::PgPool,
        id: Uuid,
        body: &mut Value,
    ) -> Result<(), AppError> {
        let v: Value = sqlx::query_scalar(
            "SELECT jsonb_build_object('org', jsonb_build_object('slug', o.slug, 'display_name', o.display_name, 'verified', o.verification_status = 'verified'),
                    'site_address', w.site_address, 'site_status', w.site_status)
               FROM wishlists w JOIN organizations o ON o.id = w.org_id WHERE w.id = $1")
            .bind(id).fetch_one(pool).await?;
        // 事件：已合併就沿 merged_into 走到最終事件（最多 10 跳）；下架 / 找不到為 null
        let event: Option<Value> = sqlx::query_scalar(
            "WITH RECURSIVE c AS (
               SELECT e.id, e.slug::text AS slug, e.name, e.merged_into, e.moderation_status::text AS m, 0 AS n
                 FROM wishlists w JOIN relief_events e ON e.id = w.event_id WHERE w.id = $1
               UNION ALL
               SELECT e.id, e.slug::text, e.name, e.merged_into, e.moderation_status::text, c.n + 1
                 FROM c JOIN relief_events e ON e.id = c.merged_into WHERE c.n < 10)
             SELECT jsonb_build_object('slug', slug, 'name', name) FROM c WHERE merged_into IS NULL AND m = 'ok'")
            .bind(id).fetch_optional(pool).await?;
        let mut v = v;
        v["event"] = event.unwrap_or(Value::Null);
        if let (Some(b), Some(x)) = (body.as_object_mut(), v.as_object()) {
            b.extend(x.clone());
        }
        Ok(())
    }

    #[derive(serde::Deserialize)]
    pub struct PageQ {
        page: Option<i64>,
    }

    pub async fn events(
        State(st): State<AppState>,
        Query(q): Query<PageQ>,
    ) -> Result<Response, AppError> {
        let page = q.page.unwrap_or(1).clamp(1, 100);
        let mut rows: Vec<Value> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT {j} FROM relief_events WHERE merged_into IS NULL AND moderation_status = 'ok'
               AND EXISTS (SELECT 1 FROM wishlists w WHERE w.event_id = relief_events.id AND {LIVE_SITE})
             ORDER BY occurred_on DESC NULLS LAST, id LIMIT $1 OFFSET $2", j = crate::relief_events::J)))
            .bind(EVENTS_PAGE + 1).bind((page - 1) * EVENTS_PAGE).fetch_all(&st.pool).await?;
        let more = rows.len() as i64 > EVENTS_PAGE;
        rows.truncate(EVENTS_PAGE as usize);
        Ok((
            CACHE,
            Json(json!({ "events": rows, "next_page": more.then_some(page + 1) })),
        )
            .into_response())
    }

    pub async fn event(
        State(st): State<AppState>,
        Path(slug): Path<String>,
    ) -> Result<Response, AppError> {
        if !crate::validate::slug_ok(&slug) {
            return Err(AppError::NotFound);
        }
        // 沿 merged_into 走到最終事件（最多 10 跳；環或過長視為不存在）
        let mut cur = slug.clone();
        let mut seen = vec![slug.clone()];
        let ev: (Uuid, String, Option<String>, String) = loop {
            let r: (Uuid, String, Option<String>, String) = sqlx::query_as(
                "SELECT e.id, e.slug::text, m.slug::text, e.moderation_status::text FROM relief_events e LEFT JOIN relief_events m ON m.id = e.merged_into WHERE e.slug = $1")
                .bind(&cur).fetch_optional(&st.pool).await?.ok_or(AppError::NotFound)?;
            match r.2.clone() {
                Some(next) if seen.len() <= 10 && !seen.contains(&next) => {
                    seen.push(next.clone());
                    cur = next;
                }
                Some(_) => return Err(AppError::NotFound),
                None => break r,
            }
        };
        if ev.3 != "ok" {
            return Err(AppError::NotFound);
        }
        if seen.len() > 1 {
            return Ok((
                CACHE,
                Json(json!({ "merged_into": ev.1, "event": null, "sites": [] })),
            )
                .into_response());
        }
        let event: Value = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT {} FROM relief_events WHERE id = $1",
            crate::relief_events::J
        )))
        .bind(ev.0)
        .fetch_one(&st.pool)
        .await?;
        // 尚未滿足：數量型 qty_claimed < qty_needed；眾籌型只有 open 算（funded / fulfilled 已達標，expired 已結束不再募）
        let sites: Vec<Value> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT jsonb_build_object('slug', w.slug, 'title', w.title, 'site_address', w.site_address, 'site_status', w.site_status,
                      'org', jsonb_build_object('slug', o.slug, 'display_name', o.display_name),
                      'needs', jsonb_build_object({needs}))
               FROM wishlists w JOIN organizations o ON o.id = w.org_id
               LEFT JOIN wishlist_items i ON i.wishlist_id = w.id AND i.deleted_at IS NULL
              WHERE w.event_id = $1 AND {LIVE_SITE}
              GROUP BY w.id, o.id ORDER BY w.created_at, w.id",
            needs = ["critical", "high", "medium", "low"].map(|u| format!(
                "'{u}', count(i.id) FILTER (WHERE i.urgency = '{u}' AND i.funding_mode <> 'support' AND CASE WHEN i.funding_mode = 'crowdfund' THEN i.funding_status = 'open' ELSE i.qty_claimed < i.qty_needed END)"
            )).join(","))))
            .bind(ev.0).fetch_all(&st.pool).await?;
        Ok((
            CACHE,
            Json(json!({ "event": event, "sites": sites, "merged_into": null })),
        )
            .into_response())
    }

    pub async fn org(
        State(st): State<AppState>,
        Path(slug): Path<String>,
    ) -> Result<Response, AppError> {
        if !crate::validate::slug_ok(&slug) {
            return Err(AppError::NotFound);
        }
        let (id, mut v): (Uuid, Value) = sqlx::query_as(
            "SELECT id, jsonb_build_object('slug', slug, 'display_name', display_name, 'intro', intro, 'website', website,
                    'public_email', public_email, 'public_phone', public_phone, 'logo_key', logo_key)
               FROM organizations WHERE slug = $1 AND verification_status = 'verified'")
            .bind(&slug).fetch_optional(&st.pool).await?.ok_or(AppError::NotFound)?;
        let key = v["logo_key"].take();
        v["logo"] = json!(key
            .as_str()
            .map(|k| crate::uploads::S3::get().public_url(k)));
        v.as_object_mut().map(|o| o.remove("logo_key"));
        let sites: Vec<Value> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT jsonb_build_object('slug', w.slug, 'title', w.title, 'site_address', w.site_address, 'site_status', w.site_status,
                      'event', jsonb_build_object('slug', e.slug, 'name', e.name))
               FROM wishlists w JOIN relief_events e ON e.id = w.event_id WHERE w.org_id = $1 AND {LIVE_SITE} ORDER BY w.created_at, w.id")))
            .bind(id).fetch_all(&st.pool).await?;
        v["sites"] = json!(sites);
        Ok((CACHE, Json(v)).into_response())
    }
}
