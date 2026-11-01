//! 清單（建立者）與品項 CRUD，全部限擁有者；非擁有者一律 NOT_FOUND。
use crate::{error::AppError, session::CurrentUser, uploads, AppState};
use axum::{
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, patch, post},
    Json, Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD as B64, Engine};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use rand::Rng;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use sqlx::PgPool;
use std::collections::HashMap;
use uuid::Uuid;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/wishlists", post(create).get(list))
        .route(
            "/wishlists/{id}",
            get(get_one).patch(update).delete(archive),
        )
        .route("/wishlists/{id}/dashboard", get(dashboard))
        .route("/wishlists/{id}/orders", get(orders))
        .route(
            "/wishlists/{id}/shipping-address",
            get(address_get).put(address_put),
        )
        .route(
            "/wishlists/{id}/allowed-users",
            get(allowed_get).put(allowed_put),
        )
        .route("/wishlists/{id}/items", post(item_create))
        .route("/wishlists/{id}/items/reorder", post(item_reorder))
        .route("/items/{item_id}", patch(item_update).delete(item_delete))
}

type R<T> = Result<T, AppError>;
fn app_url() -> String {
    std::env::var("APP_URL")
        .unwrap_or_else(|_| "http://localhost:3000".into())
        .trim_end_matches('/')
        .to_string()
}
/// 完成度唯一定義：sum(qty_claimed)/sum(qty_needed)*100 四捨五入；總需求 0 回 0。
pub fn completion_pct(claimed: i64, needed: i64) -> i64 {
    if needed <= 0 {
        0
    } else {
        (claimed * 100 + needed / 2) / needed
    }
}
fn forbidden(d: &str) -> AppError {
    AppError::problem(403, "FORBIDDEN", d)
}
fn closed() -> AppError {
    AppError::problem(409, "WISHLIST_CLOSED", "清單已關閉或封存，無法修改。")
}
fn stale() -> AppError {
    AppError::problem(
        409,
        "STALE_VERSION",
        "這份資料已在其他地方被修改，請重新載入後再試。",
    )
}
fn surprise_locked_err() -> AppError {
    forbidden("驚喜模式期間無法刪除品項或調降數量。")
}
/// 樂觀並行控制：body 的 expected_updated_at（選填）；未帶 = None，維持舊行為。
fn expected(m: &Map<String, Value>) -> R<Option<DateTime<Utc>>> {
    match m.get("expected_updated_at") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => s.parse().map(Some).map_err(|_| {
            AppError::invalid(
                "/expected_updated_at",
                "FORMAT",
                "expected_updated_at 須為 RFC 3339 時間",
            )
        }),
        _ => Err(AppError::invalid(
            "/expected_updated_at",
            "TYPE",
            "expected_updated_at 須為字串",
        )),
    }
}
fn today_tw() -> NaiveDate {
    (Utc::now() + Duration::hours(8)).date_naive()
}

/// 衍生 display_status（與 points::DISPLAY_STATUS_SQL 同義，給已載入 funding_status / 採購單狀態的 Rust 端用）。
pub fn display_status(fs: Option<&str>, order: Option<&str>) -> Option<&'static str> {
    Some(match (fs?, order) {
        ("open", _) => "open",
        ("expired", _) => "expired",
        ("fulfilled", _) | (_, Some("delivered")) => "delivered",
        (_, Some("shipped")) => "shipped",
        (_, Some("placed")) => "ordered",
        _ => "funded",
    })
}
/// 完成度的 (已完成, 需求) 單位：數量型 = (qty_claimed, qty_needed)；眾籌型 = 達標（funded / fulfilled）算 1/1，否則 0/1。
/// 刻意不用 pledged/target 換算：驚喜鎖定時完成度只會在「達標」那一刻跳動，不會隨每筆認捐洩漏進度。
pub fn units(mode: &str, needed: i32, claimed: i32, fs: Option<&str>) -> (i64, i64) {
    if mode == "support" {
        (0, 0) // 支持品項不計入完成度
    } else if mode == "crowdfund" {
        (matches!(fs, Some("funded" | "fulfilled")) as i64, 1)
    } else {
        (claimed as i64, needed as i64)
    }
}
/// 同 `units` 的 SQL 版（wishlist_items 欄位，無別名）
pub const CLAIMED_SQL: &str = "CASE WHEN funding_mode = 'support' THEN 0 WHEN funding_mode = 'crowdfund' THEN (funding_status IN ('funded', 'fulfilled'))::int ELSE qty_claimed END";
/// 同 `units` 的需求量 SQL 版（無別名）：眾籌 1、支持 0、其餘 qty_needed
pub const NEEDED_SQL: &str =
    "CASE funding_mode WHEN 'crowdfund' THEN 1 WHEN 'support' THEN 0 ELSE qty_needed END";
/// 品項是否還能送：眾籌依募資狀態與期限（同 contributions.rs 的檢查），一般品項依剩餘數量
pub const OPEN_SQL: &str = "CASE WHEN funding_mode = 'support' THEN false WHEN funding_mode = 'crowdfund' THEN funding_status = 'open' AND funding_deadline > now() ELSE qty_claimed < qty_needed END";

/// 清單封存 / 帳號刪除：把這些清單所有品項的 pledged 認捐退回錢包（captured 的採購單照常進行）。
/// 呼叫端須已在同交易 UPDATE 過 wishlists（取得清單鎖，鎖序：清單 → 錢包 → 品項 → 認捐）。
pub async fn release_pledged(tx: &mut crate::points::Tx<'_>, wishlist_ids: &[Uuid]) -> R<()> {
    let rows: Vec<(Uuid, Uuid, Uuid)> = sqlx::query_as("SELECT id, item_id, wishlist_id FROM contributions WHERE wishlist_id = ANY($1) AND status = 'pledged'")
        .bind(wishlist_ids).fetch_all(&mut **tx).await?;
    if rows.is_empty() {
        return Ok(());
    }
    let mut items: Vec<Uuid> = rows.iter().map(|r| r.1).collect();
    items.sort();
    items.dedup();
    for it in &items {
        crate::notify::enqueue_donors(&mut **tx, *it, "item.removed", &["pledged"]).await?;
    } // 先通知：release 後狀態就不是 pledged 了
    let ids: Vec<Uuid> = rows.iter().map(|r| r.0).collect();
    let released = crate::points::release(tx, &ids, &["pledged"]).await?;
    for r in &released {
        sqlx::query("UPDATE wishlist_items SET pledged_points = pledged_points - $2 WHERE id = $1")
            .bind(r.item_id)
            .bind(r.back)
            .execute(&mut **tx)
            .await?;
        sqlx::query("INSERT INTO audit_logs (actor_type, action, entity, entity_id, diff) VALUES ('system', 'contribution.release_wishlist_closed', 'contributions', $1, $2)")
            .bind(r.contribution_id).bind(json!({ "item_id": r.item_id, "points": r.back })).execute(&mut **tx).await?;
    }
    let mut ws: Vec<Uuid> = rows.iter().map(|r| r.2).collect();
    ws.sort();
    ws.dedup();
    for w in ws {
        crate::dashboard::notify(&mut **tx, w).await?;
    }
    Ok(())
}

/// 開啟生日通知（D65）：檢查擁有者生日與清單條件，同交易關掉該擁有者其他清單再設為真（不回 409）。
async fn enable_birthday(
    tx: &mut crate::points::Tx<'_>,
    id: Uuid,
    owner: Uuid,
    personal: bool,
    vis: &str,
) -> R<()> {
    let e = |code, d| AppError::invalid("/birthday_notify", code, d);
    if !personal {
        return Err(e("NOT_PERSONAL", "組織清單與救災據點不可開啟生日通知"));
    }
    if vis == "private" {
        return Err(e("LIST_PRIVATE", "私人清單不可開啟生日通知"));
    }
    let b: (Option<String>, String) =
        sqlx::query_as("SELECT birthday, birthday_visibility::text FROM users WHERE id=$1")
            .bind(owner)
            .fetch_one(&mut **tx)
            .await?;
    if b.0.is_none() {
        return Err(e("BIRTHDAY_REQUIRED", "請先在個人資料設定生日"));
    }
    if b.1 == "private" {
        return Err(e("BIRTHDAY_PRIVATE", "生日設為私人時無法通知朋友"));
    }
    sqlx::query("UPDATE wishlists SET birthday_notify=false WHERE owner_id=$1 AND id<>$2 AND birthday_notify")
        .bind(owner)
        .bind(id)
        .execute(&mut **tx)
        .await?;
    // shortcut: 同一人並行開兩份清單時後到者撞 unique index 回 500，實務幾乎不會發生；有回報再改鎖 users 列
    sqlx::query("UPDATE wishlists SET birthday_notify=true WHERE id=$1")
        .bind(id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

// ---------- 清單 ----------
#[derive(sqlx::FromRow)]
pub(crate) struct W {
    id: Uuid,
    ty: String,
    status: String,
    visibility: String,
    slug: String,
    title: String,
    description: Option<String>,
    cover_image_key: Option<String>,
    cover_image_status: String,
    cover_preset: Option<String>,
    event_date: Option<NaiveDate>,
    show_claimer_names: bool,
    surprise_mode: bool,
    surprise_locked: bool,
    claim_ttl_hours: Option<i32>,
    moderation_status: String,
    moderation_reason: Option<String>,
    moderated_at: Option<DateTime<Utc>>,
    closes_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    has_shipping_address: bool,
    org_id: Option<Uuid>,
    event_id: Option<Uuid>,
    permit_id: Option<Uuid>,
    site_address: Option<String>,
    site_status: Option<String>,
    org: Option<Value>,
    birthday_notify: bool,
}
pub(crate) const WCOLS: &str = "id, type::text AS ty, status::text AS status, visibility::text AS visibility, slug::text AS slug, title, description,
  cover_image_key, cover_image_status::text AS cover_image_status, cover_preset, event_date, show_claimer_names, surprise_mode,
  surprise_locked(surprise_mode, event_date) AS surprise_locked,
  claim_ttl_hours, moderation_status::text AS moderation_status, moderation_reason, moderated_at, closes_at, created_at, updated_at,
  EXISTS (SELECT 1 FROM shipping_addresses s WHERE s.wishlist_id = wishlists.id) AS has_shipping_address,
  org_id, event_id, permit_id, site_address, site_status, birthday_notify,
  (SELECT jsonb_build_object('id', o.id, 'slug', o.slug::text, 'display_name', o.display_name) FROM organizations o WHERE o.id = wishlists.org_id) AS org";

/// 擁有權謂詞（綁定 $2 = 使用者）：個人清單看 owner_id（保留 owner 索引），組織據點看 org_members 的角色與 disabled_at。
const MANAGE: &str = "((org_id IS NULL AND owner_id=$2) OR (org_id IS NOT NULL AND can_manage_wishlist(owner_id, org_id, $2)))";

impl W {
    pub(crate) fn json(&self) -> Value {
        let img = uploads::S3::get();
        json!({
            "id": self.id, "type": self.ty, "status": self.status, "visibility": self.visibility, "slug": self.slug,
            "title": self.title, "description": self.description,
            "cover_image_url": self.cover_image_key.as_ref().filter(|_| self.cover_image_status == "ready").map(|k| img.public_url(k)),
            "cover_image_status": self.cover_image_status, "cover_preset": self.cover_preset, "event_date": self.event_date,
            "show_claimer_names": self.show_claimer_names, "surprise_mode": self.surprise_mode, "surprise_locked": self.surprise_locked,
            "claim_ttl_hours": self.claim_ttl_hours, "moderation_status": self.moderation_status,
            "moderation_reason": self.moderation_reason, "moderated_at": self.moderated_at, "closes_at": self.closes_at,
            "has_shipping_address": self.has_shipping_address, "location": null, "address": null,
            "org_id": self.org_id, "event_id": self.event_id, "permit_id": self.permit_id, "site_address": self.site_address,
            "site_status": self.site_status, "org": self.org, "birthday_notify": self.birthday_notify,
            "share_url": format!("{}/s/{}", app_url(), self.slug), "created_at": self.created_at, "updated_at": self.updated_at,
        })
    }
}

pub(crate) async fn load_owned(pool: impl sqlx::PgExecutor<'_>, id: Uuid, user: Uuid) -> R<W> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {WCOLS} FROM wishlists WHERE id=$1 AND {MANAGE} AND deleted_at IS NULL"
    )))
    .bind(id)
    .bind(user)
    .fetch_optional(pool)
    .await?
    .ok_or(AppError::NotFound)
}

pub(crate) fn new_slug() -> String {
    const A: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
    let mut r = rand::thread_rng();
    (0..10)
        .map(|_| A[r.gen_range(0..A.len())] as char)
        .collect()
}

fn obj(v: &Value) -> R<&Map<String, Value>> {
    v.as_object()
        .ok_or_else(|| AppError::problem(400, "BAD_REQUEST", "請求內容必須是 JSON 物件"))
}

/// 讀取字串欄位：None=未帶；Some(None)=null/空字串；Some(Some)=值（trim 後，長度 min..=max 字元）
fn text(m: &Map<String, Value>, k: &str, min: usize, max: usize) -> R<Option<Option<String>>> {
    match m.get(k) {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(Value::String(s)) => {
            let s = crate::validate::text(s, &format!("/{k}"), k == "description")?;
            let s = s.as_str();
            if s.is_empty() && min == 0 {
                return Ok(Some(None));
            }
            let n = s.chars().count();
            if n < min.max(1) || n > max {
                return Err(AppError::invalid(
                    &format!("/{k}"),
                    "RANGE",
                    &format!("{k} 長度須為 {} 到 {max} 字", min.max(1)),
                ));
            }
            Ok(Some(Some(s.to_string())))
        }
        _ => Err(AppError::invalid(
            &format!("/{k}"),
            "TYPE",
            &format!("{k} 必須是字串"),
        )),
    }
}
fn boolean(m: &Map<String, Value>, k: &str) -> R<Option<bool>> {
    match m.get(k) {
        None => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        _ => Err(AppError::invalid(
            &format!("/{k}"),
            "TYPE",
            &format!("{k} 必須是布林值"),
        )),
    }
}
fn int(m: &Map<String, Value>, k: &str, lo: i64, hi: i64) -> R<Option<Option<i64>>> {
    match m.get(k) {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(v) => match v.as_i64() {
            Some(n) if (lo..=hi).contains(&n) => Ok(Some(Some(n))),
            _ => Err(AppError::invalid(
                &format!("/{k}"),
                "RANGE",
                &format!("{k} 必須介於 {lo} 與 {hi}"),
            )),
        },
    }
}
fn one_of<'a>(m: &'a Map<String, Value>, k: &str, allowed: &[&str]) -> R<Option<&'a str>> {
    match m.get(k) {
        None => Ok(None),
        Some(Value::String(s)) if allowed.contains(&s.as_str()) => Ok(Some(s)),
        _ => Err(AppError::invalid(
            &format!("/{k}"),
            "ENUM",
            &format!("{k} 必須是 {}", allowed.join(" / ")),
        )),
    }
}
const VIS: &[&str] = &["public", "friends", "selected", "private"];
fn date(m: &Map<String, Value>, k: &str) -> R<Option<Option<NaiveDate>>> {
    match m.get(k) {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(Value::String(s)) => {
            let d = s.parse().map_err(|_| {
                AppError::invalid(&format!("/{k}"), "FORMAT", "日期格式須為 YYYY-MM-DD")
            })?;
            Ok(Some(Some(crate::validate::event_date(
                d,
                &format!("/{k}"),
            )?)))
        }
        _ => Err(AppError::invalid(
            &format!("/{k}"),
            "TYPE",
            "日期格式須為 YYYY-MM-DD",
        )),
    }
}
/// closes_at：RFC 3339 時間或 null（不設期限）。「須在未來」由呼叫端檢查（`future_only`），
/// 因為編輯頁每次儲存都會帶回現有值，未變動的過期值不應擋住其他欄位的編輯。
fn closes_at(m: &Map<String, Value>) -> R<Option<Option<DateTime<Utc>>>> {
    match m.get("closes_at") {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(Value::String(s)) => {
            let t: DateTime<Utc> = s.parse().map_err(|_| {
                AppError::invalid("/closes_at", "FORMAT", "closes_at 須為 RFC 3339 時間")
            })?;
            Ok(Some(Some(t)))
        }
        _ => Err(AppError::invalid(
            "/closes_at",
            "TYPE",
            "closes_at 須為字串或 null",
        )),
    }
}
fn future_only(t: Option<DateTime<Utc>>) -> R<()> {
    if t.is_some_and(|t| t <= Utc::now()) {
        return Err(AppError::invalid(
            "/closes_at",
            "RANGE",
            "結束時間須在現在之後",
        ));
    }
    Ok(())
}
/// 預設封面 id（前端 /covers/{id}.svg）；null 清除。與上傳封面不互斥，顯示優先序由前端決定
const COVER_PRESETS: &[&str] = &[
    "birthday",
    "wedding",
    "baby",
    "housewarming",
    "graduation",
    "holiday",
    "travel",
    "gift",
];
fn cover_preset(m: &Map<String, Value>) -> R<Option<Option<String>>> {
    if matches!(m.get("cover_preset"), Some(Value::Null)) {
        return Ok(Some(None));
    }
    Ok(one_of(m, "cover_preset", COVER_PRESETS)?.map(|s| Some(s.to_string())))
}
/// 圖片 key：須為 uploads 發出的格式，且前綴符合用途
fn image_key(m: &Map<String, Value>, k: &str, dir: &str) -> R<Option<Option<String>>> {
    Ok(match text(m, k, 0, 200)? {
        Some(Some(s)) if !(uploads::valid_key(&s) && s.starts_with(dir)) => {
            return Err(AppError::invalid(
                &format!("/{k}"),
                "FORMAT",
                "圖片 key 格式不正確",
            ))
        }
        v => v,
    })
}

async fn create(u: CurrentUser, State(st): State<AppState>, Json(b): Json<Value>) -> R<Response> {
    let m = obj(&b)?;
    let ty = one_of(m, "type", &["personal", "registry", "relief"])?
        .ok_or_else(|| AppError::invalid("/type", "REQUIRED", "type 必填"))?;
    if ty == "relief" {
        return Err(AppError::invalid(
            "/type",
            "ENUM",
            "relief 據點請使用 POST /orgs/{id}/sites",
        ));
    }
    let title = text(m, "title", 1, 100)?
        .flatten()
        .ok_or_else(|| AppError::invalid("/title", "REQUIRED", "title 必填"))?;
    let desc = text(m, "description", 0, 2000)?.flatten();
    let cover = image_key(m, "cover_image_key", "covers/")?.flatten();
    let preset = cover_preset(m)?.flatten();
    let event = date(m, "event_date")?.flatten();
    let vis = one_of(m, "visibility", VIS)?.unwrap_or("friends");
    let names = boolean(m, "show_claimer_names")?.unwrap_or(false);
    let surprise = boolean(m, "surprise_mode")?.unwrap_or(false);
    // Q12：未帶欄位 = 預設購買期限 7 天；明確 null = 不限
    let ttl = int(m, "claim_ttl_hours", 1, 24 * 365)?.unwrap_or(Some(168));
    let closes = closes_at(m)?.flatten();
    future_only(closes)?;
    let bday = boolean(m, "birthday_notify")?.unwrap_or(false);
    if surprise && !event.is_some_and(|d| d > today_tw()) {
        return Err(AppError::invalid(
            "/event_date",
            "REQUIRED",
            "驚喜模式需填寫未來的活動日期",
        ));
    }
    for _ in 0..5 {
        let mut tx = uploads::media_tx(&st.pool, u.id).await?;
        uploads::remember_media(&mut tx, u.id, cover.as_deref()).await?;
        let r = sqlx::query_scalar::<_, Uuid>(
            "INSERT INTO wishlists (owner_id, type, visibility, slug, title, description, cover_image_key, cover_image_status, event_date, show_claimer_names, surprise_mode, claim_ttl_hours, closes_at, cover_preset)
             VALUES ($1,$2::wishlist_type,$3::visibility,$4,$5,$6,$7, CASE WHEN $7::text IS NULL THEN 'none' ELSE 'ready' END::image_status,$8,$9,$10,$11,$12,$13) RETURNING id")
            .bind(u.id).bind(ty).bind(vis).bind(new_slug()).bind(&title).bind(&desc).bind(&cover).bind(event).bind(names).bind(surprise).bind(ttl.map(|t| t as i32)).bind(closes).bind(&preset)
            .fetch_one(&mut *tx).await;
        match r {
            Ok(id) => {
                if bday {
                    enable_birthday(&mut tx, id, u.id, true, vis).await?;
                }
                tx.commit().await?;
                let w = load_owned(&st.pool, id, u.id).await?;
                return Ok((
                    StatusCode::CREATED,
                    [(header::LOCATION, format!("/api/v1/wishlists/{id}"))],
                    Json(w.json()),
                )
                    .into_response());
            }
            Err(sqlx::Error::Database(e)) if e.constraint() == Some("wishlists_slug_key") => {
                continue
            }
            Err(e) => return Err(e.into()),
        }
    }
    Err(AppError::problem(
        500,
        "INTERNAL_ERROR",
        "無法產生分享代碼，請重試",
    ))
}

#[derive(Deserialize)]
struct ListQ {
    cursor: Option<String>,
    limit: Option<i64>,
    status: Option<String>,
    #[serde(rename = "type")]
    ty: Option<String>,
}

async fn list(
    u: CurrentUser,
    State(st): State<AppState>,
    Query(q): Query<ListQ>,
) -> R<Json<Value>> {
    let limit = q.limit.unwrap_or(20);
    if !(1..=100).contains(&limit) {
        return Err(AppError::invalid(
            "/limit",
            "RANGE",
            "limit 必須介於 1 與 100",
        ));
    }
    // status 可用逗號列多個（如 draft,active,closed：儀表板排除已封存，分頁才不被佔位）
    let status: Option<Vec<String>> = q
        .status
        .as_deref()
        .map(|s| s.split(',').map(str::to_string).collect());
    if status.as_ref().is_some_and(|v| {
        v.iter()
            .any(|s| !["draft", "active", "closed", "archived"].contains(&s.as_str()))
    }) {
        return Err(AppError::invalid("/status", "ENUM", "status 不正確"));
    }
    if q.ty
        .as_deref()
        .is_some_and(|s| !["personal", "registry", "relief"].contains(&s))
    {
        return Err(AppError::invalid("/type", "ENUM", "type 不正確"));
    }
    let after: Option<Uuid> = match &q.cursor {
        None => None,
        Some(c) => Some(
            B64.decode(c)
                .ok()
                .and_then(|b| String::from_utf8(b).ok())
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| AppError::invalid("/cursor", "FORMAT", "cursor 無效"))?,
        ),
    };
    let rows: Vec<W> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {WCOLS} FROM wishlists WHERE owner_id=$1 AND org_id IS NULL AND deleted_at IS NULL AND ($2::uuid IS NULL OR id < $2)
         AND ($3::text[] IS NULL OR status::text = ANY($3)) AND ($4::text IS NULL OR type::text=$4) ORDER BY id DESC LIMIT $5")))
        .bind(u.id).bind(after).bind(&status).bind(&q.ty).bind(limit + 1).fetch_all(&st.pool).await?;
    let more = rows.len() as i64 > limit;
    let rows = &rows[..rows.len().min(limit as usize)];
    let ids: Vec<Uuid> = rows.iter().map(|w| w.id).collect();
    let stats: Vec<(Uuid, i64, i64, i64, i64)> = sqlx::query_as(
        sqlx::AssertSqlSafe(format!("SELECT wishlist_id, count(*), count(*) FILTER (WHERE {CLAIMED_SQL} >= qty_needed), sum({CLAIMED_SQL})::bigint, sum({NEEDED_SQL})::bigint FROM wishlist_items
         WHERE wishlist_id = ANY($1) AND deleted_at IS NULL AND funding_mode <> 'support' GROUP BY wishlist_id"))).bind(&ids).fetch_all(&st.pool).await?;
    let data: Vec<Value> = rows.iter().map(|w| {
        let (c, f, q, nd) = stats.iter().find(|s| s.0 == w.id).map(|s| (s.1, s.2, s.3, s.4)).unwrap_or((0, 0, 0, 0));
        let j = w.json();
        json!({ "id": w.id, "type": w.ty, "status": w.status, "visibility": w.visibility, "slug": w.slug, "title": w.title, "event_date": w.event_date,
                "cover_image_url": j["cover_image_url"], "cover_preset": w.cover_preset, "surprise_locked": w.surprise_locked,
                "completion": { "item_count": c, "fulfilled_count": f, "completion_pct": completion_pct(q, nd) },
                "moderation_status": w.moderation_status, "moderation_reason": w.moderation_reason,
                "birthday_notify": w.birthday_notify, "updated_at": w.updated_at })
    }).collect();
    let next = if more {
        rows.last().map(|w| B64.encode(w.id.to_string()))
    } else {
        None
    };
    Ok(Json(json!({ "data": data, "next_cursor": next })))
}

// ---------- 品項 ----------
#[derive(sqlx::FromRow)]
struct I {
    id: Uuid,
    wishlist_id: Uuid,
    title: String,
    description: Option<String>,
    brand: Option<String>,
    spec: Option<String>,
    image_key: Option<String>,
    image_status: String,
    product_url: Option<String>,
    unit_price_amount: Option<i64>,
    funding_mode: String,
    priority: String,
    qty_needed: i32,
    qty_claimed: i32,
    sort_order: i32,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    target_points: Option<i64>,
    pledged_points: i64,
    funding_status: Option<String>,
    funding_deadline: Option<DateTime<Utc>>,
    fulfillment_type: Option<String>,
    price_snapshot_amount: Option<i64>,
    expired_at: Option<DateTime<Utc>>,
    order_status: Option<String>,
    category: Option<String>,
    urgency: Option<String>,
    qty_received: i64,
    support_wishlist_id: Option<Uuid>,
}
const ICOLS: &str = "id, wishlist_id, title, description, brand, spec, image_key, image_status::text AS image_status, product_url, unit_price_amount,
  funding_mode::text AS funding_mode, priority::text AS priority, qty_needed, qty_claimed, sort_order, created_at, updated_at,
  target_points, pledged_points, funding_status::text AS funding_status, funding_deadline, fulfillment_type::text AS fulfillment_type,
  price_snapshot_amount, expired_at, (SELECT po.status::text FROM purchase_orders po WHERE po.item_id = wishlist_items.id) AS order_status,
  category, urgency, (SELECT COALESCE(SUM(r.qty), 0)::bigint FROM receipts r WHERE r.item_id = wishlist_items.id) AS qty_received, support_wishlist_id";

impl I {
    fn json(&self, locked: bool) -> Value {
        let cf = self.funding_mode == "crowdfund";
        json!({
            "id": self.id, "wishlist_id": self.wishlist_id, "title": self.title, "description": self.description, "brand": self.brand, "spec": self.spec,
            "image_url": self.image_key.as_ref().filter(|_| self.image_status == "ready").map(|k| uploads::S3::get().public_url(k)),
            "image_status": self.image_status, "product_url": self.product_url, "unit_price_amount": self.unit_price_amount,
            "funding_mode": self.funding_mode, "priority": self.priority, "category": self.category, "urgency": self.urgency,
            "qty_needed": self.qty_needed, "qty_claimed": if locked { None } else { Some(self.qty_claimed) }, "qty_received": self.qty_received, "support_wishlist_id": self.support_wishlist_id,
            // 眾籌欄位：驚喜鎖定時 pledged_points / funding_status / display_status / order_status 一律 null（否則可由「已達標」推知有人捐）
            "target_points": self.target_points, "pledged_points": if cf && !locked { Some(self.pledged_points) } else { None },
            "funding_status": self.funding_status.as_ref().filter(|_| !locked),
            "display_status": if locked { None } else { display_status(self.funding_status.as_deref(), self.order_status.as_deref()) },
            "funding_deadline": self.funding_deadline, "fulfillment_type": self.fulfillment_type,
            "catalog_product_id": null, "price_snapshot_amount": self.price_snapshot_amount, "expired_at": self.expired_at,
            "order_status": self.order_status.as_ref().filter(|_| !locked),
            "sort_order": self.sort_order, "created_at": self.created_at,
            // 捐助會推進品項 updated_at；鎖定期間遮蔽，避免由其變化推知有人捐助
            "updated_at": if locked { self.created_at } else { self.updated_at },
        })
    }
}

async fn items_of(pool: &PgPool, wid: Uuid) -> R<Vec<I>> {
    Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!("SELECT {ICOLS} FROM wishlist_items WHERE wishlist_id=$1 AND deleted_at IS NULL ORDER BY sort_order, id"))).bind(wid).fetch_all(pool).await?)
}

async fn get_one(
    u: CurrentUser,
    State(st): State<AppState>,
    Path(id): Path<Uuid>,
) -> R<Json<Value>> {
    let w = load_owned(&st.pool, id, u.id).await?;
    let items: Vec<Value> = items_of(&st.pool, id)
        .await?
        .iter()
        .map(|i| i.json(w.surprise_locked))
        .collect();
    Ok(Json(json!({ "wishlist": w.json(), "items": items })))
}

async fn update(
    u: CurrentUser,
    State(st): State<AppState>,
    Path(id): Path<Uuid>,
    Json(b): Json<Value>,
) -> R<Json<Value>> {
    let m = obj(&b)?;
    let mut tx = uploads::media_tx(&st.pool, u.id).await?;
    let mut w = load_owned(&mut *tx, id, u.id).await?;
    uploads::remember_media(&mut tx, u.id, w.cover_image_key.as_deref()).await?;
    let was_servable = crate::cdn::servable(&w.visibility, &w.status, &w.moderation_status);
    let old_shown = (w.show_claimer_names, w.surprise_mode, w.event_date); // 影響公開頁姓名顯示
    if w.status == "archived" {
        let only = |s: &str| {
            m.get("status").is_some_and(|v| v == s)
                && m.keys()
                    .all(|k| k == "status" || k == "expected_updated_at")
        };
        // 重複封存為冪等（FR-15）：只帶 status=archived（與 expected_updated_at）時直接回目前狀態
        if only("archived") {
            return Ok(Json(w.json()));
        }
        // 取消封存只能回到 closed（要再收捐助由創建者重開）；封存時退回的眾籌點數不會再扣回
        if !only("closed") {
            return Err(closed());
        }
    }
    let exp = expected(m)?;
    let relief = w.ty == "relief";
    let old_addr = w.site_address.clone();
    let (old_permit, old_site_status, old_event) = (w.permit_id, w.site_status.clone(), w.event_id); // UPDATE 時比對：並行撤銷 / 下架 / 合併改過就當 stale
    apply_site_fields(&mut tx, m, &mut w, relief).await?;
    if let Some(v) = text(m, "title", 1, 100)? {
        w.title = v.unwrap_or_default();
    }
    if let Some(v) = text(m, "description", 0, 2000)? {
        w.description = v;
    }
    if let Some(v) = image_key(m, "cover_image_key", "covers/")? {
        w.cover_image_status = if v.is_some() { "ready" } else { "none" }.into();
        w.cover_image_key = v;
    }
    if let Some(v) = cover_preset(m)? {
        w.cover_preset = v;
    }
    // 沒有活動日的舊資料永遠算鎖定，允許關閉以免卡死；用載入時的日期判斷，同一請求清掉日期不能繞過鎖定
    let date_locked = w.surprise_locked && w.event_date.is_some();
    if let Some(v) = date(m, "event_date")? {
        w.event_date = v;
    }
    if let Some(v) = one_of(m, "visibility", VIS)? {
        w.visibility = v.into();
    }
    if let Some(v) = boolean(m, "show_claimer_names")? {
        if relief && v {
            return Err(AppError::invalid(
                "/show_claimer_names",
                "NOT_APPLICABLE",
                "救災據點不顯示捐助者姓名",
            ));
        }
        w.show_claimer_names = v;
    }
    let bday = boolean(m, "birthday_notify")?.unwrap_or(false);
    if let Some(v) = int(m, "claim_ttl_hours", 1, 24 * 365)? {
        w.claim_ttl_hours = v.map(|t| t as i32);
    }
    if let Some(v) = closes_at(m)? {
        if v != w.closes_at {
            future_only(v)?;
        } // 未變動的值（含已過期）原樣放行
        w.closes_at = v;
    }
    if let Some(v) = boolean(m, "surprise_mode")? {
        if !v && date_locked {
            return Err(forbidden("驚喜鎖定期間不可關閉驚喜模式"));
        }
        w.surprise_mode = v;
    }
    if w.surprise_mode
        && !w.event_date.is_some_and(|d| d > today_tw())
        && (m.contains_key("surprise_mode") || m.contains_key("event_date"))
    {
        return Err(AppError::invalid(
            "/event_date",
            "REQUIRED",
            "驚喜模式需填寫未來的活動日期",
        ));
    }
    let was_active = w.status == "active";
    let old_status = w.status.clone(); // UPDATE 時比對：並行封存 / close_due 改過狀態就當 stale，was_active 才可信
    if let Some(new) = one_of(m, "status", &["draft", "active", "closed", "archived"])? {
        let ok = matches!((w.status.as_str(), new), (a, b) if a == b)
            || matches!(
                (w.status.as_str(), new),
                ("draft", "active")
                    | ("active", "closed")
                    | ("closed", "active")
                    | ("draft", "archived")
                    | ("active", "archived")
                    | ("closed", "archived")
                    | ("archived", "closed")
            );
        if !ok {
            return Err(AppError::invalid(
                "/status",
                "TRANSITION",
                &format!("不能從 {} 變更為 {new}", w.status),
            ));
        }
        if relief && new == "active" && w.status != "active" {
            relief_publishable(&mut tx, &w).await?;
            // 草稿 / 關閉的據點重新上線也不可掛在已下架或已合併的事件下（鎖序 organizations → relief_events）
            crate::org_sites::lock_event(&mut *tx, w.event_id.ok_or(AppError::NotFound)?).await?;
        }
        if new == "active" && w.status == "draft" {
            let n: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM wishlist_items WHERE wishlist_id=$1 AND deleted_at IS NULL",
            )
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
            let no_addr: bool = sqlx::query_scalar(
                "SELECT NOT EXISTS (SELECT 1 FROM shipping_addresses WHERE wishlist_id = $1)
                    AND EXISTS (SELECT 1 FROM wishlist_items WHERE wishlist_id = $1 AND deleted_at IS NULL AND funding_mode = 'crowdfund')")
                .bind(id).fetch_one(&mut *tx).await?;
            let mut miss = vec![];
            if no_addr {
                miss.push(json!({ "pointer": "/shipping_address", "code": "SHIPPING_ADDRESS_REQUIRED", "detail": "含眾籌品項的清單需先填寫收件資訊" }));
            }
            if w.title.trim().is_empty() {
                miss.push(
                    json!({ "pointer": "/title", "code": "REQUIRED", "detail": "標題不可為空" }),
                );
            }
            if n == 0 {
                miss.push(json!({ "pointer": "/items", "code": "REQUIRED", "detail": "至少需要 1 個品項" }));
            }
            if w.surprise_mode && !w.event_date.is_some_and(|d| d > today_tw()) {
                miss.push(json!({ "pointer": "/event_date", "code": "REQUIRED", "detail": "驚喜模式的活動日須在今日之後" }));
            }
            if !miss.is_empty() {
                return Err(AppError::Problem {
                    status: 409,
                    code: "WISHLIST_NOT_PUBLISHABLE",
                    detail: "清單尚不符合發佈條件。".into(),
                    errors: Some(Value::Array(miss)),
                    retry_after: None,
                });
            }
        }
        w.status = new.into();
    }
    // 已上線的據點換許可、或由暫停 / 關閉恢復營運：用 PATCH 後的許可重驗（暫停 / 關閉 / 改其他欄位不重驗，許可到期不該擋住收攤）
    let resumed =
        w.site_status.as_deref() == Some("active") && old_site_status.as_deref() != Some("active");
    if relief
        && old_status == "active"
        && w.status == "active"
        && (w.permit_id != old_permit || resumed)
    {
        relief_publishable(&mut tx, &w).await?;
    }
    // 重開為 active 時，建立者帳號已刪除 / 停權就改記操作者，否則認捐會因查不到 owner 而 404
    let reopening = relief && w.status == "active" && (old_status != "active" || resumed);
    // 只在「進入 active」（發佈 / 重開）時擋已過的結束時間，否則下個 tick 又被關掉；
    // 本來就 active 但已到期、尚未被背景 job 掃到的清單，編輯其他欄位照常放行（有效狀態已是 closed，捐助早被 SQL 擋住）。
    if !was_active && w.status == "active" && w.closes_at.is_some_and(|t| t <= Utc::now()) {
        return Err(AppError::invalid(
            "/closes_at",
            "RANGE",
            "結束時間已過，請延後或清除後再開啟",
        ));
    }
    uploads::remember_media(&mut tx, u.id, w.cover_image_key.as_deref()).await?;
    let res = sqlx::query("UPDATE wishlists SET title=$2, description=$3, cover_image_key=$4, cover_image_status=$5::image_status, event_date=$6, visibility=$7::visibility,
                 show_claimer_names=$8, surprise_mode=$9, claim_ttl_hours=$10, status=$11::wishlist_status, closes_at=$13,
                 birthday_notify = birthday_notify AND $7 <> 'private',
                 closes_reminded_at = CASE WHEN closes_at IS DISTINCT FROM $13 THEN NULL ELSE closes_reminded_at END, cover_preset=$15,
                 closed_at = CASE WHEN $11 IN ('closed','archived') THEN coalesce(closed_at, now()) ELSE NULL END,
                 site_status=$16, site_address=$17, event_id=$18, permit_id=$19,
                 owner_id = CASE WHEN $20::bool AND EXISTS (SELECT 1 FROM users o WHERE o.id = wishlists.owner_id AND (o.deleted_at IS NOT NULL OR o.blocked_at IS NOT NULL))
                                 THEN $21 ELSE owner_id END
                 WHERE id=$1 AND ($12::timestamptz IS NULL OR updated_at=$12) AND status=$14::wishlist_status
                   AND site_status IS NOT DISTINCT FROM $22 AND event_id IS NOT DISTINCT FROM $23")
        .bind(id).bind(&w.title).bind(&w.description).bind(&w.cover_image_key).bind(&w.cover_image_status).bind(w.event_date).bind(&w.visibility)
        .bind(w.show_claimer_names).bind(w.surprise_mode).bind(w.claim_ttl_hours).bind(&w.status).bind(exp).bind(w.closes_at).bind(&old_status).bind(&w.cover_preset)
        .bind(&w.site_status).bind(&w.site_address).bind(w.event_id).bind(w.permit_id)
        .bind(reopening).bind(u.id).bind(&old_site_status).bind(old_event).execute(&mut *tx).await?;
    if res.rows_affected() == 0 {
        return Err(stale());
    }
    if bday {
        enable_birthday(
            &mut tx,
            id,
            u.id,
            w.org_id.is_none() && !relief,
            &w.visibility,
        )
        .await?;
    } else if m.get("birthday_notify") == Some(&Value::Bool(false)) {
        sqlx::query("UPDATE wishlists SET birthday_notify=false WHERE id=$1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    if relief && m.contains_key("site_address") {
        // 收件地址 = 據點地址：眾籌採購讀 shipping_addresses，兩邊要一致
        sqlx::query("UPDATE shipping_addresses SET address_enc = $2 WHERE wishlist_id = $1")
            .bind(id)
            .bind(crate::sealed::seal(
                w.site_address.as_deref().unwrap_or_default().as_bytes(),
            ))
            .execute(&mut *tx)
            .await?;
    }
    if w.status == "archived" {
        release_pledged(&mut tx, &[id]).await?;
    }
    // 據點由營運中變為暫停 / 結束：通知進行中的捐助者
    if relief
        && old_site_status.as_deref() == Some("active")
        && w.site_status.as_deref() != Some("active")
    {
        crate::notify::enqueue_site_paused(&mut *tx, &[id]).await?;
    }
    crate::dashboard::notify(&mut *tx, id).await?; // 即時連線立即重驗可見性 / 狀態
    if was_active && matches!(w.status.as_str(), "closed" | "archived") {
        crate::claims::grace_on_close(&mut tx, id).await?; // 須在交易最後（鎖序）
    }
    tx.commit().await?;
    // 收緊可見性 / 封存後，或仍可公開但姓名顯示條件變了（副本可能還帶著捐助者姓名），邊緣快取的公開副本立即清掉
    if was_servable
        && (!crate::cdn::servable(&w.visibility, &w.status, &w.moderation_status)
            || old_shown != (w.show_claimer_names, w.surprise_mode, w.event_date))
    {
        crate::cdn::purge_wishlists(vec![w.slug.trim().to_string()]);
    }
    // 據點的公開頁與所屬事件頁（含換事件前的舊事件）會顯示營運狀態 / 地址
    if relief
        && (w.site_status != old_site_status
            || w.site_address != old_addr
            || w.event_id != old_event
            || w.status != old_status)
    {
        crate::cdn::purge_wishlists(vec![w.slug.trim().to_owned()]); // 與上面的 purge 分開寫：mutate.py 的 cdn-private 以原文唯一定位
        let evs: Vec<Uuid> = [w.event_id, old_event].into_iter().flatten().collect();
        let slugs: Vec<String> =
            sqlx::query_scalar("SELECT slug::text FROM relief_events WHERE id = ANY($1)")
                .bind(&evs)
                .fetch_all(&st.pool)
                .await
                .unwrap_or_default();
        crate::cdn::purge_events(slugs);
    }
    Ok(Json(load_owned(&st.pool, id, u.id).await?.json()))
}

/// relief 據點的 PATCH 欄位：type / visibility / org_id 不可改；site_status / site_address / event_id / permit_id 可改（非 relief 不接受）。
async fn apply_site_fields(
    tx: &mut crate::points::Tx<'_>,
    m: &Map<String, Value>,
    w: &mut W,
    relief: bool,
) -> R<()> {
    const SITE: [&str; 4] = ["site_status", "site_address", "event_id", "permit_id"];
    if !relief {
        return match SITE.iter().find(|k| m.contains_key(**k)) {
            Some(k) => Err(AppError::invalid(
                &format!("/{k}"),
                "NOT_APPLICABLE",
                "只有救災據點可設定此欄位",
            )),
            None => Ok(()),
        };
    }
    let fixed = [
        ("type", json!(w.ty)),
        ("visibility", json!(w.visibility)),
        ("org_id", json!(w.org_id)),
    ];
    if let Some((k, _)) = fixed
        .iter()
        .find(|(k, cur)| m.get(*k).is_some_and(|v| v != cur))
    {
        return Err(AppError::invalid(
            &format!("/{k}"),
            "IMMUTABLE",
            "救災據點的此欄位不可修改",
        ));
    }
    let new_status = one_of(m, "site_status", &["active", "paused", "closed"])?;
    // 恢復營運（非 active → active）：事件被下架時不可由組織自行恢復（下架會暫停據點）
    let resuming = new_status == Some("active") && w.site_status.as_deref() != Some("active");
    if let Some(v) = new_status {
        w.site_status = Some(v.into());
    }
    if let Some(v) = text(m, "site_address", 5, 200)? {
        w.site_address = Some(v.ok_or_else(|| {
            AppError::invalid("/site_address", "REQUIRED", "site_address 不可為空")
        })?);
    }
    let uuid = |k: &str| -> R<Option<Uuid>> {
        match m.get(k) {
            None => Ok(None),
            Some(Value::String(s)) => s
                .parse()
                .map(Some)
                .map_err(|_| AppError::invalid(&format!("/{k}"), "FORMAT", "須為 UUID")),
            _ => Err(AppError::invalid(
                &format!("/{k}"),
                "TYPE",
                "須為 UUID 字串",
            )),
        }
    };
    if let Some(p) = uuid("permit_id")? {
        let ok: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM org_permits WHERE id = $1 AND org_id = $2)",
        )
        .bind(p)
        .bind(w.org_id)
        .fetch_one(&mut **tx)
        .await?;
        if !ok {
            return Err(AppError::invalid(
                "/permit_id",
                "INVALID",
                "許可不屬於此組織",
            ));
        }
        w.permit_id = Some(p);
    }
    let new_event = uuid("event_id")?;
    if new_event.is_some() || resuming {
        // 鎖序 organizations → relief_events → wishlists（與上線檢查、事件合併一致）
        sqlx::query("SELECT id FROM organizations WHERE id = $1 FOR SHARE")
            .bind(w.org_id)
            .fetch_optional(&mut **tx)
            .await?;
        let e = new_event.or(w.event_id).ok_or(AppError::NotFound)?;
        crate::org_sites::lock_event(&mut **tx, e).await?;
        w.event_id = Some(e);
    }
    Ok(())
}

/// 據點上線（含重開）的應用層檢查：組織須 verified（403）、許可須 approved 且今天（台北）在效期內（409）。DB trigger 為安全網。
async fn relief_publishable(tx: &mut crate::points::Tx<'_>, w: &W) -> R<()> {
    let verified: bool = sqlx::query_scalar(
        "SELECT verification_status = 'verified' FROM organizations WHERE id = $1 FOR SHARE",
    ) // 鎖序 organizations → wishlists：與撤銷驗證互斥
    .bind(w.org_id)
    .fetch_optional(&mut **tx)
    .await?
    .unwrap_or(false);
    if !verified {
        return Err(AppError::problem(
            403,
            "ORG_NOT_VERIFIED",
            "組織尚未通過審核，據點不能上線。",
        ));
    }
    let valid: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM org_permits WHERE id = $1 AND org_id = $2 AND status = 'approved'
                        AND (now() AT TIME ZONE 'Asia/Taipei')::date BETWEEN valid_from AND valid_to)")
        .bind(w.permit_id).bind(w.org_id).fetch_one(&mut **tx).await?;
    if !valid {
        return Err(AppError::problem(
            409,
            "PERMIT_INVALID",
            "許可未核准或已過期，據點不能上線。",
        ));
    }
    Ok(())
}

/// closes_at 到期 → active 轉 closed（同手動關閉；冪等，多實例安全）。捐助 / 認捐路徑另外即時檢查 closes_at，這裡只負責狀態與快取。
pub async fn close_due(pool: &PgPool) -> Result<usize, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let rows: Vec<(Uuid, String)> = sqlx::query_as(
        "UPDATE wishlists SET status = 'closed', closed_at = now()
         WHERE status = 'active' AND closes_at <= now() RETURNING id, slug::text",
    )
    .fetch_all(&mut *tx)
    .await?;
    for (id, _) in &rows {
        crate::dashboard::notify(&mut *tx, *id).await?;
        crate::claims::grace_on_close(&mut tx, *id).await?;
    }
    tx.commit().await?;
    if !rows.is_empty() {
        crate::cdn::purge_wishlists(rows.iter().map(|r| r.1.trim().to_string()).collect());
    } // 公開快取副本的 status 要更新
    Ok(rows.len())
}

/// closes_at 前 24 小時提醒創建者一次（寄信走既有 notifications 佇列）。冪等：UPDATE ... IS NULL 保證多實例只排一封。
/// 清單剛設定且離結束不到 24 小時時，會在下個 tick 立刻提醒。
pub async fn remind_closing(pool: &PgPool) -> Result<u64, sqlx::Error> {
    Ok(sqlx::query(
        "WITH d AS (UPDATE wishlists SET closes_reminded_at = now()
                     WHERE status = 'active' AND deleted_at IS NULL AND closes_reminded_at IS NULL
                       AND closes_at > now() AND closes_at <= now() + interval '24 hours'
                     RETURNING id, owner_id, closes_at)
         INSERT INTO notifications (user_id, channel, kind, payload)
         SELECT owner_id, 'email', 'wishlist.closing', jsonb_build_object('wishlist_id', id, 'closes_at', closes_at) FROM d")
        .execute(pool).await?.rows_affected())
}

async fn archive(
    u: CurrentUser,
    State(st): State<AppState>,
    Path(id): Path<Uuid>,
) -> R<StatusCode> {
    let w = load_owned(&st.pool, id, u.id).await?;
    let mut tx = st.pool.begin().await?;
    // 交易內讀到的舊狀態才準：並行封存 / close_due 已結束的清單不再重給寬限
    let was: Option<String> = sqlx::query_scalar(
        "WITH o AS (SELECT status::text AS s FROM wishlists WHERE id=$1 AND status<>'archived' FOR UPDATE)
         UPDATE wishlists SET status='archived', closed_at = coalesce(closed_at, now()) FROM o WHERE id=$1 RETURNING o.s")
        .bind(id).fetch_optional(&mut *tx).await?;
    release_pledged(&mut tx, &[id]).await?;
    crate::dashboard::notify(&mut *tx, id).await?; // 即時連線收到後關閉
    if was.is_some_and(|s| s != "closed") {
        crate::claims::grace_on_close(&mut tx, id).await?; // 須在交易最後（鎖序）
    }
    tx.commit().await?;
    crate::cdn::purge_wishlists(vec![w.slug.trim().to_string()]);
    Ok(StatusCode::NO_CONTENT)
}

/// 各狀態採購單數量（funded=pending、ordered=placed）
fn orders_summary(statuses: impl Iterator<Item = Option<String>>) -> Value {
    let (mut f, mut o, mut s, mut d) = (0, 0, 0, 0);
    for st in statuses.flatten() {
        match st.as_str() {
            "pending" => f += 1,
            "placed" => o += 1,
            "shipped" => s += 1,
            "delivered" => d += 1,
            _ => {}
        }
    }
    json!({ "funded_count": f, "ordered_count": o, "shipped_count": s, "delivered_count": d })
}

async fn dashboard(
    u: CurrentUser,
    State(st): State<AppState>,
    Path(id): Path<Uuid>,
) -> R<Json<Value>> {
    let w = load_owned(&st.pool, id, u.id).await?;
    let items = items_of(&st.pool, id).await?;
    let locked = w.surprise_locked;
    let (mut n, mut f, mut claimed, mut needed) = (0i64, 0i64, 0i64, 0i64);
    for i in &items {
        let (c, nd) = units(
            &i.funding_mode,
            i.qty_needed,
            i.qty_claimed,
            i.funding_status.as_deref(),
        );
        if nd == 0 {
            continue;
        }
        n += 1;
        claimed += c;
        needed += nd;
        if c >= nd {
            f += 1;
        }
    }
    let qty_items = || items.iter().filter(|i| i.funding_mode == "quantity");
    let cf_items = || items.iter().filter(|i| i.funding_mode == "crowdfund");
    let unlock: Option<DateTime<Utc>> = match w.event_date {
        Some(d) if w.surprise_mode => {
            sqlx::query_scalar("SELECT ($1::date)::timestamp AT TIME ZONE 'Asia/Taipei'")
                .bind(d)
                .fetch_one(&st.pool)
                .await?
        }
        _ => None,
    };
    // 每品項幾人認領（有效 claims 筆數 / pledged+captured contributions 的不同捐贈者）：鎖定期間也輸出，只有人數、沒有身分
    let counts: HashMap<Uuid, i64> = sqlx::query_as(
        "SELECT c.item_id, count(*) FROM claims c JOIN wishlist_items i ON i.id = c.item_id
          WHERE i.wishlist_id = $1 AND c.status IN ('reserved', 'purchased', 'delivered') GROUP BY c.item_id
         UNION ALL
         SELECT item_id, count(DISTINCT user_id) FROM contributions WHERE wishlist_id = $1 AND status IN ('pledged', 'captured') GROUP BY item_id")
        .bind(id).fetch_all(&st.pool).await?.into_iter().collect();
    // 驚喜鎖定：名單一律空陣列（不輸出姓名、留言、時間、claim id）
    let claims = if locked {
        json!([])
    } else {
        type ClaimDashboardRow = (
            Uuid,
            Uuid,
            String,
            i32,
            String,
            Option<String>,
            DateTime<Utc>,
            bool,
        );
        let rows: Vec<ClaimDashboardRow> = sqlx::query_as(
            "SELECT c.id, c.item_id, CASE WHEN c.is_anonymous THEN '匿名朋友' ELSE c.claimer_name END, c.qty, c.status::text, c.note, c.created_at, c.is_anonymous FROM claims c JOIN wishlist_items i ON i.id=c.item_id
             WHERE i.wishlist_id=$1 ORDER BY c.id").bind(id).fetch_all(&st.pool).await?;
        Value::Array(
            rows.into_iter()
                .map(|r| {
                    json!({ "id": r.0, "item_id": r.1,
            "claimer_name": r.2, "qty": r.3, "status": r.4, "note": r.5, "created_at": r.6, "is_anonymous": r.7 })
                })
                .collect(),
        )
    };
    // 捐贈者名單（受捐者的核心需求）：誰、捐多少、何時捐、何時達標（captured_at）。只列 pledged / captured；
    // 匿名者一律顯示「匿名朋友」，不輸出 email 或 user_id。驚喜鎖定時為空陣列。
    let (contributions, funded_at) = if locked {
        (json!([]), HashMap::new())
    } else {
        type Row = (
            Uuid,
            Uuid,
            i64,
            i64,
            Option<String>,
            String,
            String,
            DateTime<Utc>,
            Option<DateTime<Utc>>,
            Option<DateTime<Utc>>,
            bool,
        );
        let rows: Vec<Row> = sqlx::query_as(
            "SELECT c.id, c.item_id, c.points, c.refunded_points, c.message, c.status::text,
                    CASE WHEN c.is_anonymous THEN '匿名朋友' ELSE c.donor_name END, c.created_at, c.captured_at, c.released_at, c.is_anonymous
               FROM contributions c JOIN wishlist_items i ON i.id = c.item_id
              WHERE c.wishlist_id = $1 AND i.deleted_at IS NULL AND c.status IN ('pledged', 'captured')
              ORDER BY c.created_at, c.id").bind(id).fetch_all(&st.pool).await?;
        let mut at: HashMap<Uuid, DateTime<Utc>> = HashMap::new();
        for r in &rows {
            if let Some(t) = r.8 {
                at.entry(r.1).and_modify(|e| *e = (*e).min(t)).or_insert(t);
            }
        }
        (Value::Array(rows.into_iter().map(|r| json!({ "id": r.0, "item_id": r.1, "points": r.2, "refunded_points": r.3, "spent_points": r.2 - r.3,
            "message": r.4, "status": r.5, "display_name": r.6, "created_at": r.7, "captured_at": r.8, "released_at": r.9, "is_anonymous": r.10 })).collect()), at)
    };
    let (target_sum, pledged_sum) = (
        cf_items().filter_map(|i| i.target_points).sum::<i64>(),
        cf_items().map(|i| i.pledged_points).sum::<i64>(),
    );
    let funded = cf_items()
        .filter(|i| matches!(i.funding_status.as_deref(), Some("funded" | "fulfilled")))
        .count();
    Ok(Json(json!({
        "wishlist_id": id, "surprise_locked": locked, "unlock_at": unlock,
        "moderation_status": w.moderation_status, "moderation_reason": w.moderation_reason,
        "totals": { "item_count": n, "fulfilled_count": f, "completion_pct": completion_pct(claimed, needed),
            "qty_needed": qty_items().map(|i| i.qty_needed as i64).sum::<i64>(),
            "qty_claimed": qty_items().map(|i| i.qty_claimed as i64).sum::<i64>(),
            "target_points": target_sum, "pledged_points": if locked { None } else { Some(pledged_sum) },
            "funded_item_count": if locked { None } else { Some(funded) } },
        "items": items.iter().map(|i| {
            let cf = i.funding_mode == "crowdfund";
            json!({ "item_id": i.id, "title": i.title, "funding_mode": i.funding_mode, "qty_needed": i.qty_needed,
                "qty_claimed": i.qty_claimed, "claimer_count": counts.get(&i.id).copied().unwrap_or(0),
                "target_points": i.target_points, "pledged_points": cf.then_some(i.pledged_points),
                "funding_status": i.funding_status.as_ref().filter(|_| !locked),
                "display_status": if locked { None } else { display_status(i.funding_status.as_deref(), i.order_status.as_deref()) },
                "funding_deadline": i.funding_deadline, "funded_at": funded_at.get(&i.id) })
        }).collect::<Vec<_>>(),
        "orders_summary": orders_summary(cf_items().map(|i| i.order_status.clone())),
        "claims": claims, "contributions": contributions,
    })))
}

/// GET /wishlists/{id}/orders：達標品項的採購單進度。驚喜鎖定時 data 為 null，只回彙總。
async fn orders(
    u: CurrentUser,
    State(st): State<AppState>,
    Path(id): Path<Uuid>,
) -> R<Json<Value>> {
    let w = load_owned(&st.pool, id, u.id).await?;
    type Row = (
        Uuid,
        Uuid,
        String,
        String,
        String,
        i64,
        Option<String>,
        Option<DateTime<Utc>>,
        Option<DateTime<Utc>>,
        Option<DateTime<Utc>>,
        DateTime<Utc>,
    );
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT po.id, po.item_id, i.title, po.fulfillment_type::text, po.status::text, po.amount, po.tracking_no, po.placed_at, po.shipped_at, po.delivered_at, po.updated_at
           FROM purchase_orders po JOIN wishlist_items i ON i.id = po.item_id
          WHERE i.wishlist_id = $1 AND i.deleted_at IS NULL ORDER BY po.created_at, po.id").bind(id).fetch_all(&st.pool).await?;
    let summary = orders_summary(rows.iter().map(|r| Some(r.4.clone())));
    let data = if w.surprise_locked {
        Value::Null
    } else {
        Value::Array(rows.into_iter().map(|r| json!({ "id": r.0, "item_id": r.1, "item_title": r.2, "fulfillment_type": r.3, "status": r.4, "amount": r.5,
            "tracking_no": r.6, "placed_at": r.7, "shipped_at": r.8, "delivered_at": r.9, "updated_at": r.10 })).collect())
    };
    Ok(Json(
        json!({ "surprise_locked": w.surprise_locked, "summary": summary, "data": data }),
    ))
}

// ---------- 收件資訊（欄位級加密，明文不進 log）----------
fn stars(n: usize) -> String {
    "*".repeat(n)
}
fn mask_name(s: &str) -> String {
    let c: Vec<char> = s.chars().collect();
    match c.len() {
        0 | 1 => "*".into(),
        2 => format!("{}*", c[0]),
        n => format!("{}{}{}", c[0], stars(n - 2), c[n - 1]),
    }
}
fn mask_phone(s: &str) -> String {
    let c: Vec<char> = s.chars().collect();
    let n = c.len();
    if n >= 8 {
        format!(
            "{}{}{}",
            c[..4].iter().collect::<String>(),
            stars(n - 7),
            c[n - 3..].iter().collect::<String>()
        )
    } else if n > 3 {
        format!("{}{}", stars(n - 3), c[n - 3..].iter().collect::<String>())
    } else {
        stars(n)
    }
}
fn mask_address(s: &str) -> String {
    let n = s.chars().count();
    format!(
        "{}***",
        s.chars()
            .take(if n > 6 { 6 } else { n / 2 })
            .collect::<String>()
    )
}
fn masked(name: &str, phone: &str, addr: &str) -> Value {
    json!({ "has_shipping_address": true, "recipient_name": mask_name(name), "phone": mask_phone(phone), "address": mask_address(addr) })
}

async fn address_get(
    u: CurrentUser,
    State(st): State<AppState>,
    Path(id): Path<Uuid>,
) -> R<Json<Value>> {
    load_owned(&st.pool, id, u.id).await?;
    let row: Option<(Vec<u8>, Vec<u8>, Vec<u8>)> = sqlx::query_as("SELECT recipient_name_enc, phone_enc, address_enc FROM shipping_addresses WHERE wishlist_id = $1")
        .bind(id).fetch_optional(&st.pool).await?;
    let Some((n, p, a)) = row else {
        return Ok(Json(json!({ "has_shipping_address": false })));
    };
    let open = |b: &[u8]| {
        crate::sealed::open_str(b)
            .ok_or_else(|| AppError::problem(500, "INTERNAL_ERROR", "收件資訊無法解密"))
    };
    Ok(Json(masked(&open(&n)?, &open(&p)?, &open(&a)?)))
}

async fn address_put(
    u: CurrentUser,
    State(st): State<AppState>,
    Path(id): Path<Uuid>,
    Json(b): Json<Value>,
) -> R<Json<Value>> {
    let m = obj(&b)?;
    let w = load_owned(&st.pool, id, u.id).await?;
    if w.ty == "relief" {
        return Err(AppError::invalid(
            "/type",
            "NOT_APPLICABLE",
            "救災據點的收件地址請改用 PATCH site_address",
        ));
    }
    if matches!(w.status.as_str(), "closed" | "archived") {
        return Err(closed());
    }
    let req = |k: &str, min, max| {
        text(m, k, min, max)?
            .flatten()
            .ok_or_else(|| AppError::invalid(&format!("/{k}"), "REQUIRED", &format!("{k} 必填")))
    };
    let (name, phone, addr) = (
        req("recipient_name", 1, 50)?,
        req("phone", 6, 20)?,
        req("address", 5, 200)?,
    );
    check_phone(&phone, "/phone")?;
    save_address(&st.pool, id, &name, &phone, &addr).await?;
    Ok(Json(masked(&name, &phone, &addr)))
}

/// 電話只允許數字、+、-、空白（address_put 與據點建立共用）
pub(crate) fn check_phone(phone: &str, pointer: &str) -> R<()> {
    if !phone
        .chars()
        .all(|c| c.is_ascii_digit() || matches!(c, '+' | '-' | ' '))
    {
        return Err(AppError::invalid(
            pointer,
            "FORMAT",
            "電話只能包含數字、+、- 與空白",
        ));
    }
    Ok(())
}
/// 收件資訊欄位級加密後 upsert（address_put 與據點建立共用，眾籌 capture_and_order 讀同一張表）
pub(crate) async fn save_address(
    ex: impl sqlx::PgExecutor<'_>,
    id: Uuid,
    name: &str,
    phone: &str,
    addr: &str,
) -> R<()> {
    sqlx::query("INSERT INTO shipping_addresses (wishlist_id, recipient_name_enc, phone_enc, address_enc) VALUES ($1, $2, $3, $4)
                 ON CONFLICT (wishlist_id) DO UPDATE SET recipient_name_enc = EXCLUDED.recipient_name_enc, phone_enc = EXCLUDED.phone_enc, address_enc = EXCLUDED.address_enc")
        .bind(id).bind(crate::sealed::seal(name.as_bytes())).bind(crate::sealed::seal(phone.as_bytes())).bind(crate::sealed::seal(addr.as_bytes()))
        .execute(ex).await?;
    Ok(())
}

// ---------- 品項 handlers ----------
fn http_url(s: &str) -> bool {
    reqwest::Url::parse(s).is_ok_and(|u| matches!(u.scheme(), "http" | "https"))
}

fn apply_item(m: &Map<String, Value>, i: &mut I) -> R<()> {
    if let Some(v) = text(m, "title", 1, 120)? {
        i.title = v.unwrap_or_default();
    }
    if let Some(v) = text(m, "description", 0, 2000)? {
        i.description = v;
    }
    if let Some(v) = text(m, "brand", 0, 100)? {
        i.brand = v;
    }
    if let Some(v) = text(m, "spec", 0, 200)? {
        i.spec = v;
    }
    if let Some(v) = text(m, "product_url", 0, 2000)? {
        if v.as_deref().is_some_and(|u| !http_url(u)) {
            return Err(AppError::invalid(
                "/product_url",
                "FORMAT",
                "product_url 必須是 http(s) 網址",
            ));
        }
        i.product_url = v;
    }
    if let Some(v) = int(m, "unit_price_amount", 0, 100_000_000)? {
        i.unit_price_amount = v;
    }
    if let Some(v) = one_of(m, "priority", &["high", "medium", "low"])? {
        i.priority = v.into();
    }
    if let Some(v) = int(m, "qty_needed", 1, 9999)? {
        i.qty_needed = v
            .ok_or_else(|| AppError::invalid("/qty_needed", "REQUIRED", "qty_needed 不可為 null"))?
            as i32;
    }
    if let Some(v) = image_key(m, "image_key", "items/")? {
        i.image_status = if v.is_some() { "ready" } else { "none" }.into();
        i.image_key = v;
    }
    Ok(())
}

// ---------- 眾籌品項（P2-A）----------
const MAX_TARGET: i64 = 100_000_000;

fn funding_locked() -> AppError {
    AppError::problem(
        409,
        "FUNDING_LOCKED",
        "品項已達標或已過期，不能再修改商品資訊、目標點數或截止時間。",
    )
}
fn address_required() -> AppError {
    AppError::problem(
        409,
        "SHIPPING_ADDRESS_REQUIRED",
        "請先填寫清單的收件資訊，才能建立眾籌品項。",
    )
}

/// funding_deadline（ISO 8601）；省略 / null 時用 event_date 當天 23:59（台北時間），兩者皆無 → 422。必須在未來。
fn deadline_of(m: &Map<String, Value>, event: Option<NaiveDate>) -> R<DateTime<Utc>> {
    let dl = match m.get("funding_deadline") {
        None | Some(Value::Null) => {
            let d = event.ok_or_else(|| {
                AppError::invalid(
                    "/funding_deadline",
                    "REQUIRED",
                    "請填寫截止時間，或先設定清單的活動日期",
                )
            })?;
            d.and_hms_opt(23, 59, 0).expect("23:59").and_utc() - Duration::hours(8)
        }
        Some(Value::String(s)) => s.parse::<DateTime<Utc>>().map_err(|_| {
            AppError::invalid(
                "/funding_deadline",
                "FORMAT",
                "funding_deadline 須為 ISO 8601 時間",
            )
        })?,
        _ => {
            return Err(AppError::invalid(
                "/funding_deadline",
                "TYPE",
                "funding_deadline 須為 ISO 8601 時間",
            ))
        }
    };
    if dl <= Utc::now() || dl > Utc::now() + Duration::days(3650) {
        return Err(AppError::invalid(
            "/funding_deadline",
            "RANGE",
            "截止時間必須在未來（且不超過 10 年）",
        ));
    }
    Ok(dl)
}

async fn has_address(ex: impl sqlx::PgExecutor<'_>, wid: Uuid) -> R<bool> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM shipping_addresses WHERE wishlist_id = $1)",
    )
    .bind(wid)
    .fetch_one(ex)
    .await?)
}

/// 把 body 的 target_points / funding_deadline / price_snapshot_amount 套到一個新的（或剛切換成）眾籌品項。
async fn init_crowdfund(
    pool: impl sqlx::PgExecutor<'_>,
    m: &Map<String, Value>,
    i: &mut I,
    event: Option<NaiveDate>,
) -> R<()> {
    let target = int(m, "target_points", 1, MAX_TARGET)?
        .flatten()
        .ok_or_else(|| {
            AppError::invalid(
                "/target_points",
                "REQUIRED",
                "眾籌品項必須填寫 target_points",
            )
        })?;
    let dl = deadline_of(m, event)?;
    if let Some(p) = int(m, "price_snapshot_amount", 0, 100_000_000)? {
        i.price_snapshot_amount = p;
    }
    if !has_address(pool, i.wishlist_id).await? {
        return Err(address_required());
    }
    i.funding_mode = "crowdfund".into();
    i.qty_needed = 1; // 伺服器強制：眾籌品項固定 1 件、營運代購
    i.target_points = Some(target);
    i.funding_deadline = Some(dl);
    i.funding_status = Some("open".into());
    i.fulfillment_type = Some("concierge".into());
    Ok(())
}

/// relief 品項 category / urgency 必填（建立時），其餘清單不接受；在 app 層擋下，避免落到 trigger 回 500。
fn apply_relief(m: &Map<String, Value>, i: &mut I, relief: bool, create: bool) -> R<()> {
    const CAT: &[&str] = &[
        "water", "food", "medical", "hygiene", "shelter", "clothing", "baby", "other",
    ];
    const URG: &[&str] = &["critical", "high", "medium", "low"];
    for (k, allowed, slot) in [
        ("category", CAT, &mut i.category),
        ("urgency", URG, &mut i.urgency),
    ] {
        if !relief {
            if m.contains_key(k) {
                return Err(AppError::invalid(
                    &format!("/{k}"),
                    "NOT_APPLICABLE",
                    &format!("只有救災據點的品項可設定 {k}"),
                ));
            }
        } else if m.get(k).is_some_and(Value::is_null) {
            return Err(AppError::invalid(
                &format!("/{k}"),
                "REQUIRED",
                &format!("{k} 必填，不可清空"),
            ));
        } else if let Some(v) = one_of(m, k, allowed)? {
            *slot = Some(v.into());
        } else if create {
            return Err(AppError::invalid(
                &format!("/{k}"),
                "REQUIRED",
                &format!("{k} 必填"),
            ));
        }
    }
    Ok(())
}

async fn item_create(
    u: CurrentUser,
    State(st): State<AppState>,
    Path(wid): Path<Uuid>,
    Json(b): Json<Value>,
) -> R<Response> {
    let m = obj(&b)?;
    let mut tx = uploads::media_tx(&st.pool, u.id).await?;
    let w = load_owned(&mut *tx, wid, u.id).await?;
    if matches!(w.status.as_str(), "closed" | "archived") {
        return Err(closed());
    }
    if !m.get("title").is_some_and(|t| t.is_string()) {
        return Err(AppError::invalid("/title", "REQUIRED", "title 必填"));
    }
    let mode =
        one_of(m, "funding_mode", &["quantity", "crowdfund", "support"])?.unwrap_or("quantity");
    let mut i = I {
        id: Uuid::nil(),
        wishlist_id: wid,
        title: String::new(),
        description: None,
        brand: None,
        spec: None,
        image_key: None,
        image_status: "none".into(),
        product_url: None,
        unit_price_amount: None,
        funding_mode: "quantity".into(),
        priority: "medium".into(),
        qty_needed: 1,
        qty_claimed: 0,
        sort_order: 0,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        target_points: None,
        pledged_points: 0,
        funding_status: None,
        funding_deadline: None,
        fulfillment_type: None,
        price_snapshot_amount: None,
        expired_at: None,
        order_status: None,
        category: None,
        urgency: None,
        qty_received: 0,
        support_wishlist_id: None,
    };
    apply_item(m, &mut i)?;
    apply_relief(m, &mut i, w.ty == "relief", true)?;
    // 僅建立時可帶；PATCH 不處理（沿用 apply_item 外的欄位即忽略）
    let catalog_ref = text(m, "catalog_ref", 0, 64)?.flatten();
    if catalog_ref
        .as_deref()
        .is_some_and(|r| !crate::inspire::valid_ref(r))
    {
        return Err(AppError::invalid(
            "/catalog_ref",
            "FORMAT",
            "catalog_ref 格式不正確",
        ));
    }
    if mode == "crowdfund" {
        init_crowdfund(&mut *tx, m, &mut i, w.event_date).await?;
    } else if mode == "support" {
        let slug = text(m, "support_slug", 1, 64)?
            .flatten()
            .ok_or_else(|| AppError::invalid("/support_slug", "REQUIRED", "support_slug 必填"))?;
        crate::ratelimit::check(&mut *tx, &format!("support_item:{}", u.id), 20, 3600).await?;
        i.support_wishlist_id = Some(crate::support::resolve_target(&mut tx, &slug, u.id).await?);
        i.funding_mode = "support".into();
    }
    uploads::remember_media(&mut tx, u.id, i.image_key.as_deref()).await?;
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO wishlist_items (wishlist_id, title, description, brand, spec, image_key, image_status, product_url, unit_price_amount, priority, qty_needed, sort_order,
                funding_mode, target_points, funding_status, funding_deadline, fulfillment_type, price_snapshot_amount, catalog_ref, category, urgency, support_wishlist_id)
         VALUES ($1,$2,$3,$4,$5,$6,$7::image_status,$8,$9,$10::item_priority,$11,
                 (SELECT coalesce(max(sort_order),0)+10 FROM wishlist_items WHERE wishlist_id=$1),
                 $12::funding_mode,$13,$14::funding_status,$15,$16::fulfillment_type,$17,$18,$19,$20,$21) RETURNING id")
        .bind(wid).bind(&i.title).bind(&i.description).bind(&i.brand).bind(&i.spec).bind(&i.image_key).bind(&i.image_status)
        .bind(&i.product_url).bind(i.unit_price_amount).bind(&i.priority).bind(i.qty_needed)
        .bind(&i.funding_mode).bind(i.target_points).bind(&i.funding_status).bind(i.funding_deadline).bind(&i.fulfillment_type).bind(i.price_snapshot_amount).bind(&catalog_ref).bind(&i.category).bind(&i.urgency).bind(i.support_wishlist_id)
        .fetch_one(&mut *tx).await?;
    tx.commit().await?;
    let row: I = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {ICOLS} FROM wishlist_items WHERE id=$1"
    )))
    .bind(id)
    .fetch_one(&st.pool)
    .await?;
    Ok((StatusCode::CREATED, Json(row.json(w.surprise_locked))).into_response())
}

/// 取得擁有者的品項及其清單狀態
async fn owned_item(pool: &PgPool, item: Uuid, user: Uuid) -> R<(I, String, bool)> {
    let i: Option<I> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {ICOLS} FROM wishlist_items WHERE id=$1 AND deleted_at IS NULL AND wishlist_id IN (SELECT id FROM wishlists WHERE {MANAGE} AND deleted_at IS NULL)")))
        .bind(item).bind(user).fetch_optional(pool).await?;
    let i = i.ok_or(AppError::NotFound)?;
    let w = load_owned(pool, i.wishlist_id, user).await?;
    Ok((i, w.status, w.surprise_locked))
}

/// PATCH 的眾籌部分（呼叫端已 FOR UPDATE 鎖住品項）：funding_mode 切換、target_points / funding_deadline / price_snapshot_amount 修改。
/// 驚喜鎖定期間一律回通用 403（任何 409 都會洩漏「已有人捐 / 已達標」）。
async fn apply_funding(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    m: &Map<String, Value>,
    i: &mut I,
    event: Option<NaiveDate>,
    locked: bool,
) -> R<()> {
    let want = one_of(m, "funding_mode", &["quantity", "crowdfund"])?;
    if m.contains_key("support_slug") {
        return Err(AppError::problem(
            422,
            "SUPPORT_TARGET_LOCKED",
            "支持對象建立後不可變更。",
        ));
    }
    if i.funding_mode == "support" && want.is_some_and(|w| w != "support") {
        return Err(AppError::problem(
            409,
            "FUNDING_MODE_LOCKED",
            "支持品項無法變更募集方式。",
        ));
    }
    let touches = ["target_points", "funding_deadline", "price_snapshot_amount"]
        .iter()
        .any(|k| m.contains_key(*k));
    if let Some(mode) = want.filter(|w| *w != i.funding_mode) {
        if locked {
            return Err(surprise_locked_err());
        }
        // 已有捐助或認捐（含曾經有過）就不能換模式
        let used: bool = sqlx::query_scalar(
            "SELECT $2 > 0 OR $3 > 0 OR EXISTS (SELECT 1 FROM contributions WHERE item_id = $1)
                 OR EXISTS (SELECT 1 FROM claims WHERE item_id = $1 AND status IN ('reserved', 'purchased', 'delivered'))")
            .bind(i.id).bind(i.pledged_points).bind(i.qty_claimed as i64).fetch_one(&mut **tx).await?;
        if used {
            return Err(AppError::problem(
                409,
                "FUNDING_MODE_LOCKED",
                "品項已有人認領或贊助，無法變更募集方式。",
            ));
        }
        if mode == "crowdfund" {
            init_crowdfund(&mut **tx, m, i, event).await?;
        } else {
            i.funding_mode = "quantity".into();
            (
                i.target_points,
                i.funding_status,
                i.funding_deadline,
                i.fulfillment_type,
                i.price_snapshot_amount,
            ) = (None, None, None, None, None);
        }
    } else if i.funding_mode == "crowdfund" {
        i.qty_needed = 1;
        if !touches {
            return Ok(());
        }
        if locked {
            return Err(surprise_locked_err());
        }
        let open = i.funding_status.as_deref() == Some("open");
        if m.contains_key("target_points") {
            let t = int(m, "target_points", 1, MAX_TARGET)?
                .flatten()
                .ok_or_else(|| {
                    AppError::invalid("/target_points", "REQUIRED", "target_points 不可為 null")
                })?;
            if Some(t) != i.target_points {
                if !open {
                    return Err(funding_locked());
                }
                // 等於已認捐點數 = 視同達標，但達標只能由認捐觸發，所以直接拒絕
                if t <= i.pledged_points {
                    return Err(AppError::invalid(
                        "/target_points",
                        "RANGE",
                        "target_points 必須大於已贊助點數",
                    ));
                }
                i.target_points = Some(t);
            }
        }
        if m.contains_key("funding_deadline") {
            let d = deadline_of(m, event)?;
            if Some(d) != i.funding_deadline {
                if !open {
                    return Err(funding_locked());
                }
                i.funding_deadline = Some(d);
            }
        }
        if let Some(p) = int(m, "price_snapshot_amount", 0, 100_000_000)? {
            i.price_snapshot_amount = p;
        }
    }
    Ok(())
}

async fn item_update(
    u: CurrentUser,
    State(st): State<AppState>,
    Path(id): Path<Uuid>,
    Json(b): Json<Value>,
) -> R<Json<Value>> {
    let m = obj(&b)?;
    let (_, status, locked) = owned_item(&st.pool, id, u.id).await?;
    if matches!(status.as_str(), "closed" | "archived") {
        return Err(closed());
    }
    let mut tx = uploads::media_tx(&st.pool, u.id).await?;
    // 鎖住品項後再讀：funding 規則（已有認捐 / pledged_points）要看到最新值，並與捐助 / 認捐的條件式 UPDATE 互斥
    let mut i: I = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {ICOLS} FROM wishlist_items WHERE id=$1 AND deleted_at IS NULL FOR UPDATE"
    )))
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(AppError::NotFound)?;
    let (event, ty): (Option<NaiveDate>, String) =
        sqlx::query_as("SELECT event_date, type::text FROM wishlists WHERE id=$1")
            .bind(i.wishlist_id)
            .fetch_one(&mut *tx)
            .await?;
    uploads::remember_media(&mut tx, u.id, i.image_key.as_deref()).await?;
    let old_qty = i.qty_needed;
    // 鎖定期間品項的 updated_at 被遮蔽（見 I::json），故不做品項版本檢查
    let exp = if locked { None } else { expected(m)? };
    let product = |i: &I| {
        (
            i.title.clone(),
            i.product_url.clone(),
            i.image_key.clone(),
            i.description.clone(),
            i.brand.clone(),
            i.spec.clone(),
            i.unit_price_amount,
            i.price_snapshot_amount,
        )
    };
    let before = product(&i);
    // Q13：捐助者看得到的內容（排序、圖片處理狀態不算）；同值重送不算修改
    let visible = |i: &I| {
        (
            product(i),
            i.priority.clone(),
            i.qty_needed,
            i.funding_mode.clone(),
            i.target_points,
            i.funding_deadline,
            i.fulfillment_type.clone(),
        )
    };
    let seen = visible(&i);
    let frozen = i.funding_mode == "crowdfund" && i.funding_status.as_deref() != Some("open");
    apply_item(m, &mut i)?;
    apply_relief(m, &mut i, ty == "relief", false)?;
    apply_funding(&mut tx, m, &mut i, event, locked).await?;
    // 契約：眾籌品項 funded 之後唯讀；後台採購單即時讀這些欄位，不可事後改商品。同值重送不算修改。
    if frozen && product(&i) != before {
        return Err(if locked {
            surprise_locked_err()
        } else {
            funding_locked()
        });
    }
    // 驚喜鎖定：不論有無捐助，一律禁止調降（否則 QTY_BELOW_CLAIMED 會洩漏「有人捐助」）
    if locked && i.qty_needed < old_qty {
        return Err(surprise_locked_err());
    }
    uploads::remember_media(&mut tx, u.id, i.image_key.as_deref()).await?;
    let changed = visible(&i) != seen;
    // qty_needed >= qty_claimed 在同一 UPDATE 內判斷，避免與捐助競爭
    let res = sqlx::query("UPDATE wishlist_items SET content_changed_at = CASE WHEN $19 THEN now() ELSE content_changed_at END, title=$2, description=$3, brand=$4, spec=$5, image_key=$6, image_status=$7::image_status, product_url=$8,
                 unit_price_amount=$9, priority=$10::item_priority, qty_needed=$11, funding_mode=$13::funding_mode, target_points=$14,
                 funding_status=$15::funding_status, funding_deadline=$16, fulfillment_type=$17::fulfillment_type, price_snapshot_amount=$18, category=$20, urgency=$21
                 WHERE id=$1 AND qty_claimed <= $11 AND ($12::timestamptz IS NULL OR updated_at=$12)")
        .bind(id).bind(&i.title).bind(&i.description).bind(&i.brand).bind(&i.spec).bind(&i.image_key).bind(&i.image_status).bind(&i.product_url)
        .bind(i.unit_price_amount).bind(&i.priority).bind(i.qty_needed).bind(exp)
        .bind(&i.funding_mode).bind(i.target_points).bind(&i.funding_status).bind(i.funding_deadline).bind(&i.fulfillment_type).bind(i.price_snapshot_amount).bind(changed).bind(&i.category).bind(&i.urgency)
        .execute(&mut *tx).await?;
    if res.rows_affected() == 0 {
        let cur: Option<(DateTime<Utc>, i32)> = sqlx::query_as(
            "SELECT updated_at, qty_claimed FROM wishlist_items WHERE id=$1 AND deleted_at IS NULL",
        )
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?;
        return Err(match cur {
            None => AppError::NotFound,
            Some((u, _)) if exp.is_some_and(|e| e != u) => stale(),
            _ => AppError::problem(409, "QTY_BELOW_CLAIMED", "qty_needed 不可小於已認領數量。"),
        });
    }
    if changed {
        crate::notify::enqueue_item_changed(&mut *tx, id).await?;
    }
    crate::dashboard::notify(&mut *tx, i.wishlist_id).await?;
    let row: I = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {ICOLS} FROM wishlist_items WHERE id=$1"
    )))
    .bind(id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Json(row.json(locked)))
}

#[derive(Deserialize)]
struct ForceQ {
    force: Option<bool>,
}

async fn item_delete(
    u: CurrentUser,
    State(st): State<AppState>,
    Path(id): Path<Uuid>,
    Query(q): Query<ForceQ>,
) -> R<StatusCode> {
    let (item, _, locked) = owned_item(&st.pool, id, u.id).await?;
    let wid = item.wishlist_id;
    // 驚喜鎖定：一律禁止刪除，回應與有無捐助無關
    if locked {
        return Err(surprise_locked_err());
    }
    let mut tx = st.pool.begin().await?;
    // 清單 → 錢包 → 品項 → 捐助：所有品項都與進行中的捐助 / 認捐互斥。
    sqlx::query("SELECT id FROM wishlists WHERE id=$1 FOR UPDATE")
        .bind(wid)
        .execute(&mut *tx)
        .await?;
    // 持清單列鎖後再讀狀態：closed / archived 不可增刪品項（F8），與新增 / 修改品項一致
    let status: String = sqlx::query_scalar("SELECT status::text FROM wishlists WHERE id=$1")
        .bind(wid)
        .fetch_one(&mut *tx)
        .await?;
    if matches!(status.as_str(), "closed" | "archived") {
        return Err(closed());
    }
    if item.funding_mode == "crowdfund" {
        let (captured, pledged): (i64, i64) = sqlx::query_as(
            "SELECT count(*) FILTER (WHERE status = 'captured'), count(*) FILTER (WHERE status = 'pledged') FROM contributions WHERE item_id = $1")
            .bind(id).fetch_one(&mut *tx).await?;
        if captured > 0 {
            return Err(AppError::problem(
                409,
                "ITEM_HAS_CLAIMS",
                "品項已達標並進入採購，無法刪除。",
            ));
        }
        if pledged > 0 {
            if !q.force.unwrap_or(false) {
                return Err(AppError::problem(
                    409,
                    "ITEM_HAS_CLAIMS",
                    "品項已有進行中的贊助，需帶 force=true 才能刪除。",
                ));
            }
            let ids: Vec<Uuid> = sqlx::query_scalar(
                "SELECT id FROM contributions WHERE item_id = $1 AND status = 'pledged'",
            )
            .bind(id)
            .fetch_all(&mut *tx)
            .await?;
            crate::notify::enqueue_donors(&mut *tx, id, "item.removed", &["pledged"]).await?; // 先通知：release 後狀態就不是 pledged 了
            for r in crate::points::release(&mut tx, &ids, &["pledged"]).await? {
                sqlx::query("INSERT INTO audit_logs (actor_type, actor_id, action, entity, entity_id, diff) VALUES ('user'::actor_type, $1, 'contribution.release_by_item_delete', 'contributions', $2, $3)")
                    .bind(u.id).bind(r.contribution_id).bind(json!({ "points": r.back, "item_id": id })).execute(&mut *tx).await?;
            }
        }
    }
    // 與 apply / expire_due 共用 item → claim 鎖序，避免強制取消時互鎖。
    sqlx::query("SELECT id FROM wishlist_items WHERE id=$1 FOR UPDATE")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    let active: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM claims WHERE item_id=$1 AND status IN ('reserved','purchased')",
    )
    .bind(id)
    .fetch_one(&mut *tx)
    .await?;
    if active > 0 {
        if !q.force.unwrap_or(false) {
            return Err(AppError::problem(
                409,
                "ITEM_HAS_CLAIMS",
                "品項已有進行中的認領，需帶 force=true 才能刪除。",
            ));
        }
        // F-09：被取消的捐助逐筆寫 audit、通知有 email 的捐助者（中性內容）
        let cancelled: Vec<(Uuid, i32, Option<Uuid>, Option<Uuid>)> = sqlx::query_as(
            "UPDATE claims SET status='cancelled', cancelled_at=now() WHERE item_id=$1 AND status IN ('reserved','purchased')
             RETURNING id, qty, guest_id, user_id").bind(id).fetch_all(&mut *tx).await?;
        for (cid, qty, _, _) in &cancelled {
            sqlx::query("INSERT INTO audit_logs (actor_type, actor_id, action, entity, entity_id, diff) VALUES ('user'::actor_type, $1, 'claim.cancel_by_item_delete', 'claims', $2, $3)")
                .bind(u.id).bind(cid).bind(json!({ "qty": qty, "item_id": id })).execute(&mut *tx).await?;
        }
        let guests: Vec<Uuid> = cancelled.iter().filter_map(|c| c.2).collect();
        let users: Vec<Uuid> = cancelled.iter().filter_map(|c| c.3).collect();
        sqlx::query("INSERT INTO notifications (guest_id, channel, kind, payload)
                     SELECT id, 'email', 'claim.item_removed', jsonb_build_object('wishlist_id', $2::uuid) FROM guests
                      WHERE id = ANY($1) AND email IS NOT NULL AND deleted_at IS NULL")
            .bind(&guests).bind(wid).execute(&mut *tx).await?;
        // 會員一律進站內信（email 與否由 worker 判斷）
        for uid in &users {
            crate::notify::enqueue(
                &mut *tx,
                *uid,
                "claim.item_removed",
                json!({"wishlist_id": wid, "item_id": id}),
            )
            .await?;
        }
    }
    sqlx::query("UPDATE wishlist_items SET deleted_at=now(),
                 qty_claimed=COALESCE((SELECT sum(qty) FROM claims WHERE item_id=$1 AND status IN ('reserved','purchased','delivered')),0),
                 pledged_points=0 WHERE id=$1").bind(id).execute(&mut *tx).await?;
    crate::dashboard::notify(&mut *tx, wid).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct Reorder {
    item_ids: Vec<Uuid>,
}

async fn item_reorder(
    u: CurrentUser,
    State(st): State<AppState>,
    Path(wid): Path<Uuid>,
    Json(r): Json<Reorder>,
) -> R<StatusCode> {
    let w = load_owned(&st.pool, wid, u.id).await?;
    if matches!(w.status.as_str(), "closed" | "archived") {
        return Err(closed());
    }
    let mut have: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM wishlist_items WHERE wishlist_id=$1 AND deleted_at IS NULL",
    )
    .bind(wid)
    .fetch_all(&st.pool)
    .await?;
    let mut want = r.item_ids.clone();
    have.sort();
    want.sort();
    if have != want {
        return Err(AppError::invalid(
            "/item_ids",
            "MISMATCH",
            "item_ids 必須與清單現有品項完全一致",
        ));
    }
    sqlx::query("UPDATE wishlist_items i SET sort_order = t.ord::int * 10 FROM unnest($1::uuid[]) WITH ORDINALITY AS t(id, ord) WHERE i.id = t.id")
        .bind(&r.item_ids).execute(&st.pool).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------- 指定對象（visibility=selected 的名單，只能是擁有者的好友） ----------
async fn allowed_list(pool: &PgPool, id: Uuid) -> R<Json<Value>> {
    let rows: Vec<(Uuid, String, Option<String>)> = sqlx::query_as(
        "SELECT u.id, u.display_name, u.handle FROM wishlist_allowed_users a JOIN users u ON u.id = a.user_id
         WHERE a.wishlist_id = $1 ORDER BY a.created_at, u.id").bind(id).fetch_all(pool).await?;
    Ok(Json(
        json!({ "users": rows.into_iter().map(|r| json!({ "id": r.0, "display_name": r.1, "handle": r.2 })).collect::<Vec<_>>() }),
    ))
}

/// 據點恆為 public，沒有指定名單
fn no_relief(w: &W) -> R<()> {
    if w.ty == "relief" {
        return Err(AppError::invalid(
            "/type",
            "NOT_APPLICABLE",
            "救災據點沒有指定名單",
        ));
    }
    Ok(())
}

async fn allowed_get(
    u: CurrentUser,
    State(st): State<AppState>,
    Path(id): Path<Uuid>,
) -> R<Json<Value>> {
    no_relief(&load_owned(&st.pool, id, u.id).await?)?;
    allowed_list(&st.pool, id).await
}

async fn allowed_put(
    u: CurrentUser,
    State(st): State<AppState>,
    Path(id): Path<Uuid>,
    Json(b): Json<Value>,
) -> R<Json<Value>> {
    no_relief(&load_owned(&st.pool, id, u.id).await?)?;
    let mut ids: Vec<Uuid> = obj(&b)?
        .get("user_ids")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .ok_or_else(|| AppError::invalid("/user_ids", "TYPE", "user_ids 必須是 uuid 陣列"))?;
    ids.sort();
    ids.dedup();
    if ids.len() > 200 {
        return Err(AppError::invalid("/user_ids", "RANGE", "最多 200 位"));
    }
    let (a, b2): (Vec<Uuid>, Vec<Uuid>) = ids
        .iter()
        .map(|&f| if u.id < f { (u.id, f) } else { (f, u.id) })
        .unzip();
    let ok: i64 = sqlx::query_scalar("SELECT count(*) FROM friendships WHERE (user_a, user_b) IN (SELECT * FROM unnest($1::uuid[], $2::uuid[]))")
        .bind(&a).bind(&b2).fetch_one(&st.pool).await?;
    if ok != ids.len() as i64 {
        return Err(AppError::problem(422, "NOT_FRIEND", "名單只能選擇你的好友"));
    }
    let mut tx = st.pool.begin().await?;
    sqlx::query("DELETE FROM wishlist_allowed_users WHERE wishlist_id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO wishlist_allowed_users (wishlist_id, user_id) SELECT $1, unnest($2::uuid[])",
    )
    .bind(id)
    .bind(&ids)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    allowed_list(&st.pool, id).await
}
