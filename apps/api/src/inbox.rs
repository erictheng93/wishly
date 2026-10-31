//! 站內通知中心：只回本人的通知（notifications.inbox_at 不為空者，由 notify worker 在 render 後寫入）。
use crate::{error::AppError, session::CurrentUser, AppState};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/me/notifications", get(list))
        .route("/me/notifications/unread-count", get(unread_count))
        .route("/me/notifications/read-all", post(read_all))
        .route("/me/notifications/{id}/read", post(read_one))
}

#[derive(Deserialize)]
struct ListQ {
    before: Option<Uuid>,
    limit: Option<i64>,
}

async fn unread(pool: &sqlx::PgPool, uid: Uuid) -> Result<i64, AppError> {
    Ok(sqlx::query_scalar("SELECT count(*) FROM notifications WHERE user_id = $1 AND inbox_at IS NOT NULL AND read_at IS NULL")
        .bind(uid).fetch_one(pool).await?)
}

async fn list(State(st): State<AppState>, u: CurrentUser, Query(q): Query<ListQ>) -> Result<Json<Value>, AppError> {
    let limit = q.limit.unwrap_or(20);
    if !(1..=50).contains(&limit) {
        return Err(AppError::invalid("/limit", "RANGE", "limit 須為 1–50"));
    }
    // 多取一筆判斷有沒有下一頁；游標是上一頁最後一筆的 id（依 inbox_at DESC, id DESC）
    let rows: Vec<(Uuid, String, Option<String>, Option<String>, chrono::DateTime<chrono::Utc>, Option<chrono::DateTime<chrono::Utc>>)> = sqlx::query_as(
        "SELECT n.id, n.kind, n.inbox_title, n.inbox_link, n.inbox_at, n.read_at FROM notifications n
          WHERE n.user_id = $1 AND n.inbox_at IS NOT NULL
            AND ($2::uuid IS NULL OR (n.inbox_at, n.id) < (SELECT b.inbox_at, b.id FROM notifications b WHERE b.id = $2 AND b.user_id = $1))
          ORDER BY n.inbox_at DESC, n.id DESC LIMIT $3")
        .bind(u.id).bind(q.before).bind(limit + 1).fetch_all(&st.pool).await?;
    let more = rows.len() as i64 > limit;
    let items: Vec<Value> = rows.iter().take(limit as usize)
        .map(|(id, kind, title, link, at, read)| json!({ "id": id, "kind": kind, "title": title, "link": link, "created_at": at, "read_at": read }))
        .collect();
    let next_cursor = if more { items.last().map(|i| i["id"].clone()) } else { None };
    Ok(Json(json!({ "items": items, "next_cursor": next_cursor, "unread": unread(&st.pool, u.id).await? })))
}

async fn unread_count(State(st): State<AppState>, u: CurrentUser) -> Result<Json<Value>, AppError> {
    Ok(Json(json!({ "unread": unread(&st.pool, u.id).await? })))
}

async fn read_one(State(st): State<AppState>, u: CurrentUser, Path(id): Path<Uuid>) -> Result<StatusCode, AppError> {
    // 已讀過的保留原時間；非本人（或不在站內信）一律 404
    let found: Option<Uuid> = sqlx::query_scalar(
        "UPDATE notifications SET read_at = COALESCE(read_at, now()) WHERE id = $1 AND user_id = $2 AND inbox_at IS NOT NULL RETURNING id")
        .bind(id).bind(u.id).fetch_optional(&st.pool).await?;
    found.map(|_| StatusCode::NO_CONTENT).ok_or(AppError::NotFound)
}

async fn read_all(State(st): State<AppState>, u: CurrentUser) -> Result<StatusCode, AppError> {
    sqlx::query("UPDATE notifications SET read_at = now() WHERE user_id = $1 AND inbox_at IS NOT NULL AND read_at IS NULL")
        .bind(u.id).execute(&st.pool).await?;
    Ok(StatusCode::NO_CONTENT)
}
