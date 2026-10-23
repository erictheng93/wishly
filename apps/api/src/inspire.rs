//! 靈感頁：GET /inspire/{ref}/wishes —— 某精選商品（catalog_ref）在「我看得到的」清單裡的願望。
//! 可見性與 friends::feed / access::check_wishlist 同規則；訪客只看得到 public。依觀看者而異，永遠 no-store。
use crate::{error::AppError, AppState};
use axum::{
    extract::{Path, Query, State},
    http::{header, request::Parts},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

pub fn routes() -> Router<AppState> {
    Router::new().route("/inspire/{ref}/wishes", get(wishes))
}

/// catalog_ref 格式：^[A-Za-z0-9_-]{1,64}$
pub fn valid_ref(r: &str) -> bool {
    (1..=64).contains(&r.len())
        && r.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

#[derive(Deserialize)]
struct Q {
    scope: Option<String>,
}

type Row = (
    Uuid,
    String,
    i32,
    i32,
    bool,
    String,
    String,
    String,
    Uuid,
    String,
    Option<String>,
);

async fn wishes(
    State(st): State<AppState>,
    Path(r): Path<String>,
    Query(q): Query<Q>,
    parts: Parts,
) -> Result<Response, AppError> {
    if !valid_ref(&r) {
        return Err(AppError::invalid("/ref", "FORMAT", "ref 格式不正確"));
    }
    if q.scope
        .as_deref()
        .is_some_and(|s| !matches!(s, "public" | "friends" | "selected"))
    {
        return Err(AppError::invalid(
            "/scope",
            "ENUM",
            "scope 必須是 public / friends / selected",
        ));
    }
    let viewer = crate::access::viewer(&parts, &st).await?;
    let rows: Vec<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT i.id, i.title, i.qty_needed, ({claimed})::int AS got, ({open}) AS open,
                w.slug::text, w.title, w.visibility::text, o.id, o.display_name, o.handle
         FROM wishlist_items i
         JOIN wishlists w ON w.id = i.wishlist_id
         JOIN users o ON o.id = w.owner_id AND o.deleted_at IS NULL AND o.blocked_at IS NULL
         WHERE i.catalog_ref = $1 AND i.deleted_at IS NULL
           AND w.deleted_at IS NULL AND w.moderation_status = 'ok' AND w.status = 'active'
           AND (w.closes_at IS NULL OR w.closes_at > now())
           AND w.owner_id IS DISTINCT FROM $2
           AND ($3::text IS NULL OR w.visibility::text = $3)
           AND (w.visibility::text = 'public'
                OR (w.visibility::text = 'friends' AND EXISTS (
                      SELECT 1 FROM friendships f WHERE f.user_a = least($2, w.owner_id) AND f.user_b = greatest($2, w.owner_id)))
                OR (w.visibility::text = 'selected' AND EXISTS (
                      SELECT 1 FROM wishlist_allowed_users a WHERE a.wishlist_id = w.id AND a.user_id = $2)))
         ORDER BY open DESC, i.created_at DESC LIMIT 50",
        claimed = crate::wishlists::CLAIMED_SQL,
        open = crate::wishlists::OPEN_SQL
    )))
    .bind(&r)
    .bind(viewer)
    .bind(q.scope)
    .fetch_all(&st.pool)
    .await?;
    let wishes: Vec<Value> = rows
        .into_iter()
        .map(|(id, title, need, got, open, slug, wt, vis, oid, on, oh)| {
            json!({
                "item": { "id": id, "title": title, "qty_needed": need, "qty_claimed": got, "open": open },
                "wishlist": { "slug": slug, "title": wt, "visibility": vis },
                "owner": { "id": oid, "display_name": on, "handle": oh } })
        })
        .collect();
    Ok((
        [(header::CACHE_CONTROL, "no-store")],
        Json(json!({ "wishes": wishes })),
    )
        .into_response())
}
