//! 站內通知中心：只回本人的通知（notifications.inbox_at 不為空者，由 notify worker 在 render 後寫入）。
use crate::{error::AppError, session::CurrentUser, AppState};
use axum::{
    extract::{Path, Query, State},
    http::{request::Parts, StatusCode},
    response::{sse::Event, Response},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/me/notifications", get(list))
        .route("/me/notifications/stream", get(stream))
        .route("/me/notifications/unread-count", get(unread_count))
        .route("/me/notifications/read-all", post(read_all))
        .route("/me/notifications/{id}/read", post(read_one))
}

#[derive(Deserialize)]
struct ListQ {
    before: Option<Uuid>,
    limit: Option<i64>,
    category: Option<String>,
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
    if q.category.as_deref().is_some_and(|c| !matches!(c, "account" | "gifts" | "social")) {
        return Err(AppError::invalid("/category", "FORMAT", "category 須為 account / gifts / social"));
    }
    // 多取一筆判斷有沒有下一頁；游標是上一頁最後一筆的 id（依 inbox_at DESC, id DESC）
    let rows: Vec<(Uuid, String, Option<String>, Option<String>, Option<String>, Option<String>, chrono::DateTime<chrono::Utc>, Option<chrono::DateTime<chrono::Utc>>)> = sqlx::query_as(
        "SELECT n.id, n.kind, n.inbox_title, n.inbox_link, n.inbox_body, n.inbox_category, n.inbox_at, n.read_at FROM notifications n
          WHERE n.user_id = $1 AND n.inbox_at IS NOT NULL
            AND ($4::text IS NULL OR n.inbox_category = $4)
            AND ($2::uuid IS NULL OR (n.inbox_at, n.id) < (SELECT b.inbox_at, b.id FROM notifications b WHERE b.id = $2 AND b.user_id = $1))
          ORDER BY n.inbox_at DESC, n.id DESC LIMIT $3")
        .bind(u.id).bind(q.before).bind(limit + 1).bind(&q.category).fetch_all(&st.pool).await?;
    let more = rows.len() as i64 > limit;
    let items: Vec<Value> = rows.iter().take(limit as usize)
        .map(|(id, kind, title, link, body, category, at, read)| json!({ "id": id, "kind": kind, "title": title, "link": link, "body": body, "category": category, "created_at": at, "read_at": read }))
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
    found.ok_or(AppError::NotFound)?;
    ping(&st, u.id).await;
    Ok(StatusCode::NO_CONTENT)
}

async fn read_all(State(st): State<AppState>, u: CurrentUser) -> Result<StatusCode, AppError> {
    sqlx::query("UPDATE notifications SET read_at = now() WHERE user_id = $1 AND inbox_at IS NOT NULL AND read_at IS NULL")
        .bind(u.id).execute(&st.pool).await?;
    ping(&st, u.id).await;
    Ok(StatusCode::NO_CONTENT)
}

/// 通知同帳號的其他分頁／裝置更新；訊號只是提示，失敗不影響主要操作。
async fn ping(st: &AppState, uid: Uuid) {
    let _ = crate::dashboard::notify(&st.pool, uid).await;
}

/// 即時訊號：只送 `event: inbox`、`data: {}`，內容由 REST 取得。連線最長 30 分鐘，客戶端重連時重新驗證 session。
async fn stream(State(st): State<AppState>, u: CurrentUser, parts: Parts) -> Result<Response, AppError> {
    let slot = crate::dashboard::SseSlot::acquire(crate::config::client_ip(&parts, &crate::config::get()))?;
    let mut rx = crate::dashboard::subscribe(&st.pool).await?;
    let (tx, out) = tokio::sync::mpsc::channel::<Result<Event, std::convert::Infallible>>(16);
    tokio::spawn(async move {
        let _slot = slot;
        let deadline = tokio::time::sleep(std::time::Duration::from_secs(30 * 60));
        tokio::pin!(deadline);
        loop {
            tokio::select! {
                _ = &mut deadline => break,
                _ = tx.closed() => break,
                r = rx.recv() => match r {
                    Ok((id, _)) if id != u.id => continue,
                    Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        if tx.send(Ok(Event::default().event("inbox").data("{}"))).await.is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                },
            }
        }
    });
    Ok(crate::dashboard::sse_response(out))
}
