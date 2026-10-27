//! 救災據點建立與列表（spec 2026-10-27-org-relief-registration-design）。
//! 據點就是 type='relief' 的 wishlist；建立後的編輯 / 品項 / 捐助都走既有 /wishlists、/items 端點，
//! 擁有權由 `can_manage_wishlist`（org_members 的 owner / coordinator）判斷。
use crate::{
    error::AppError,
    orgs::{audit_user, require_role},
    session::CurrentUser,
    validate,
    wishlists::{check_phone, load_owned, new_slug, save_address, W, WCOLS},
    AppState,
};
use axum::{
    extract::{Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::PgExecutor;
use uuid::Uuid;

type R<T> = Result<T, AppError>;

pub fn routes() -> Router<AppState> {
    Router::new().route("/orgs/{id}/sites", get(list).post(create))
}

/// 據點掛上 / 回到事件前，鎖事件列（FOR SHARE，與事件合併 / 下架的 FOR UPDATE 互斥）並確認可用：
/// 存在、未被合併、未下架，否則 422 EVENT_UNAVAILABLE。鎖序：organizations → relief_events → wishlists。
pub async fn lock_event<'e>(ex: impl PgExecutor<'e>, id: Uuid) -> R<()> {
    let ev: Option<(Option<Uuid>, String)> =
        sqlx::query_as("SELECT merged_into, moderation_status::text FROM relief_events WHERE id = $1 FOR SHARE")
            .bind(id).fetch_optional(ex).await?;
    match ev {
        Some((None, m)) if m != "hidden" => Ok(()),
        _ => Err(AppError::problem(422, "EVENT_UNAVAILABLE", "事件不存在、已合併或已下架。")),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewSite {
    title: Option<String>,
    description: Option<String>,
    event_id: Option<Uuid>,
    permit_id: Option<Uuid>,
    site_address: Option<String>,
    recipient_name: Option<String>,
    recipient_phone: Option<String>,
    claim_ttl_hours: Option<i64>,
}

/// 必填字串：trim、字元檢查、長度 min..=max
fn req(v: Option<String>, k: &str, min: usize, max: usize, multi: bool) -> R<String> {
    let p = format!("/{k}");
    let s = validate::text(&v.ok_or_else(|| AppError::invalid(&p, "REQUIRED", "必填"))?, &p, multi)?;
    let n = s.chars().count();
    if n < min.max(1) || n > max {
        return Err(AppError::invalid(&p, "RANGE", &format!("長度須為 {} 到 {max} 字", min.max(1))));
    }
    Ok(s)
}

async fn create(u: CurrentUser, State(st): State<AppState>, Path(org): Path<Uuid>, Json(b): Json<NewSite>) -> R<Response> {
    let title = req(b.title, "title", 1, 100, false)?;
    let description = b.description.map(|d| validate::text(&d, "/description", true)).transpose()?.filter(|d| !d.is_empty());
    if description.as_deref().is_some_and(|d| d.chars().count() > 2000) {
        return Err(AppError::invalid("/description", "RANGE", "description 至多 2000 字"));
    }
    let site_address = req(b.site_address, "site_address", 5, 200, false)?;
    let name = req(b.recipient_name, "recipient_name", 1, 50, false)?;
    let phone = req(b.recipient_phone, "recipient_phone", 6, 20, false)?;
    check_phone(&phone, "/recipient_phone")?;
    let ttl = b.claim_ttl_hours.unwrap_or(48);
    if !(1..=24 * 365).contains(&ttl) {
        return Err(AppError::invalid("/claim_ttl_hours", "RANGE", "claim_ttl_hours 必須介於 1 與 8760"));
    }
    let event = b.event_id.ok_or_else(|| AppError::invalid("/event_id", "REQUIRED", "必填"))?;
    let permit = b.permit_id.ok_or_else(|| AppError::invalid("/permit_id", "REQUIRED", "必填"))?;

    for _ in 0..5 {
        let mut tx = st.pool.begin().await?;
        require_role(&mut *tx, org, u.id, "coordinator").await?;
        // 草稿階段只要求許可屬於本組織且已核准；效期留到上線（PATCH status=active）才檢查
        let ps: Option<String> = sqlx::query_scalar("SELECT status FROM org_permits WHERE id = $1 AND org_id = $2")
            .bind(permit).bind(org).fetch_optional(&mut *tx).await?;
        match ps.as_deref() {
            None => return Err(AppError::invalid("/permit_id", "INVALID", "許可不屬於此組織")),
            Some("approved") => {}
            Some(_) => return Err(AppError::problem(409, "PERMIT_INVALID", "許可尚未核准")),
        }
        lock_event(&mut *tx, event).await?;
        let r = sqlx::query_scalar::<_, Uuid>(
            "INSERT INTO wishlists (owner_id, type, visibility, status, slug, title, description, claim_ttl_hours, org_id, event_id, permit_id, site_address, site_status)
             VALUES ($1,'relief','public','draft',$2,$3,$4,$5,$6,$7,$8,$9,'active') RETURNING id")
            .bind(u.id).bind(new_slug()).bind(&title).bind(&description).bind(ttl as i32).bind(org).bind(event).bind(permit).bind(&site_address)
            .fetch_one(&mut *tx).await;
        let id = match r {
            Ok(id) => id,
            Err(sqlx::Error::Database(e)) if e.constraint() == Some("wishlists_slug_key") => continue,
            Err(e) => return Err(e.into()),
        };
        // 收件地址 = 據點地址，眾籌採購（capture_and_order）沿用同一張 shipping_addresses
        save_address(&mut *tx, id, &name, &phone, &site_address).await?;
        audit_user(&mut *tx, u.id, "site.create", "wishlists", Some(id), json!({"org_id": org, "event_id": event, "permit_id": permit})).await?;
        tx.commit().await?;
        let w = load_owned(&st.pool, id, u.id).await?;
        return Ok((StatusCode::CREATED, [(header::LOCATION, format!("/api/v1/wishlists/{id}"))], Json(w.json())).into_response());
    }
    Err(AppError::problem(500, "INTERNAL_ERROR", "無法產生分享代碼，請重試"))
}

async fn list(u: CurrentUser, State(st): State<AppState>, Path(org): Path<Uuid>) -> R<Json<Value>> {
    require_role(&st.pool, org, u.id, "volunteer").await?;
    let rows: Vec<W> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {WCOLS} FROM wishlists WHERE org_id = $1 AND deleted_at IS NULL ORDER BY id DESC")))
        .bind(org).fetch_all(&st.pool).await?;
    Ok(Json(Value::Array(rows.iter().map(W::json).collect())))
}
