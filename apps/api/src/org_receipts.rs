//! 據點實收登記：任何未停用成員（volunteer 起）登記品項實際收到的數量。
//! 只 INSERT receipts（append-only），不鎖也不動 wishlist_items / claims，因此不參與認捐鎖序。
use crate::{
    error::AppError,
    idempotency,
    orgs::{audit_user, require_role},
    session::CurrentUser,
    AppState,
};
use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::Response,
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

const MAX_QTY: i32 = 100_000;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/orgs/{id}/receipts", post(record))
        .route("/orgs/{id}/sites/{wishlist_id}/receipts", get(list))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Req {
    item_id: Uuid,
    qty: i32,
    note: Option<String>,
}

fn resp(status: u16, body: &Value, replay: bool) -> Response {
    let mut b = Response::builder()
        .status(StatusCode::from_u16(status).unwrap())
        .header(header::CACHE_CONTROL, "private, no-store")
        .header(header::CONTENT_TYPE, "application/json");
    if replay {
        b = b.header("Idempotency-Replayed", HeaderValue::from_static("true"));
    }
    b.body(axum::body::Body::from(body.to_string())).unwrap()
}

async fn record(
    State(st): State<AppState>,
    me: CurrentUser,
    Path(org): Path<Uuid>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, AppError> {
    require_role(&st.pool, org, me.id, "volunteer").await?;
    let key = idempotency::key(&headers)?;
    let req: Req = serde_json::from_slice(&body).map_err(|_| AppError::invalid("/", "INVALID", "請求內容格式錯誤"))?;
    if !(1..=MAX_QTY).contains(&req.qty) {
        return Err(AppError::invalid("/qty", "RANGE", "qty 必須介於 1 與 100000"));
    }
    let note = req.note.as_deref().map(str::trim).filter(|n| !n.is_empty());
    if note.is_some_and(|n| n.chars().count() > 200) {
        return Err(AppError::invalid("/note", "RANGE", "備註至多 200 字"));
    }

    let scope = format!("user:{}:POST /orgs/{org}/receipts", me.id);
    let hash = idempotency::request_hash(&format!("POST /orgs/{org}/receipts"), &body);
    let mut tx = st.pool.begin().await?;
    if let idempotency::Begin::Replay(s, b) = idempotency::begin(&mut tx, &scope, key, &hash).await? {
        return Ok(resp(s as u16, &b, true));
    }
    let site: (Uuid, String) = sqlx::query_as(
        "SELECT w.id, w.site_status FROM wishlist_items i JOIN wishlists w ON w.id = i.wishlist_id
         WHERE i.id = $1 AND i.deleted_at IS NULL AND w.type = 'relief' AND w.org_id = $2 AND w.deleted_at IS NULL")
        .bind(req.item_id).bind(org).fetch_optional(&mut *tx).await?.ok_or(AppError::NotFound)?;
    if site.1 == "closed" {
        return Err(AppError::problem(409, "SITE_CLOSED", "此據點已結束，無法再登記實收"));
    }
    let (id, created_at): (Uuid, chrono::DateTime<chrono::Utc>) = sqlx::query_as(
        "INSERT INTO receipts (item_id, wishlist_id, qty, note, recorded_by) VALUES ($1,$2,$3,$4,$5) RETURNING id, created_at")
        .bind(req.item_id).bind(site.0).bind(req.qty).bind(note).bind(me.id).fetch_one(&mut *tx).await?;
    audit_user(&mut *tx, me.id, "receipt.create", "receipt", Some(id),
        json!({"org_id": org, "wishlist_id": site.0, "item_id": req.item_id, "qty": req.qty})).await?;
    let out = json!({"id": id, "item_id": req.item_id, "qty": req.qty, "note": note, "created_at": created_at});
    idempotency::finish(&mut tx, &scope, key, 201, &out).await?;
    tx.commit().await?;
    Ok(resp(201, &out, false))
}

async fn list(
    State(st): State<AppState>,
    me: CurrentUser,
    Path((org, wid)): Path<(Uuid, Uuid)>,
) -> Result<Json<Vec<Value>>, AppError> {
    require_role(&st.pool, org, me.id, "volunteer").await?;
    sqlx::query_scalar::<_, i32>("SELECT 1 FROM wishlists WHERE id=$1 AND org_id=$2 AND type='relief' AND deleted_at IS NULL")
        .bind(wid).bind(org).fetch_optional(&st.pool).await?.ok_or(AppError::NotFound)?;
    let rows = sqlx::query_scalar(
        "SELECT jsonb_build_object('item_id', r.item_id, 'total', sum(r.qty), 'entries', jsonb_agg(jsonb_build_object(
            'qty', r.qty, 'note', r.note, 'recorded_by_name', u.display_name, 'created_at', r.created_at)
            ORDER BY r.created_at DESC, r.id DESC))
         FROM receipts r JOIN users u ON u.id = r.recorded_by WHERE r.wishlist_id = $1
         GROUP BY r.item_id ORDER BY min(r.created_at)")
        .bind(wid).fetch_all(&st.pool).await?;
    Ok(Json(rows))
}
