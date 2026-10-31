//! 站內通知中心：只回本人的通知（notifications.inbox_at 不為空者，由 notify worker 在 render 後寫入）。
use crate::{error::AppError, session::CurrentUser, AppState};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{sse::Event, Response},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use tokio::sync::{Semaphore, SemaphorePermit};
use uuid::Uuid;

/// 站內通知串流上限。有登入所以按使用者算，不看 IP：行動網路常共用對外 IP（CGNAT），按 IP 會讓無關的人互相排擠；
/// 也與公開頁 SSE 分開計算。背景分頁會斷線，6 條足夠。實測見 docs/notification-policy.md。
const MAX_STREAMS_TOTAL: usize = 4000;
const MAX_STREAMS_PER_USER: u32 = 6;
static STREAMS_TOTAL: Semaphore = Semaphore::const_new(MAX_STREAMS_TOTAL);
static STREAMS_PER_USER: Mutex<Option<HashMap<Uuid, u32>>> = Mutex::new(None);
static STREAMS_REJECTED: AtomicU64 = AtomicU64::new(0);

/// 持有期間佔用一個全域名額與該使用者一個計數；drop 時歸還。
struct StreamSlot {
    _permit: SemaphorePermit<'static>,
    uid: Uuid,
}

impl StreamSlot {
    fn acquire(uid: Uuid) -> Result<Self, AppError> {
        let reject = |e| {
            STREAMS_REJECTED.fetch_add(1, Ordering::Relaxed);
            e
        };
        let permit = STREAMS_TOTAL
            .try_acquire()
            .map_err(|_| reject(AppError::problem(503, "SSE_CAPACITY", "即時更新連線已滿，請稍後再試")))?;
        let mut m = STREAMS_PER_USER.lock().unwrap();
        let n = m.get_or_insert_with(HashMap::new).entry(uid).or_insert(0);
        if *n >= MAX_STREAMS_PER_USER {
            return Err(reject(AppError::problem(429, "RATE_LIMITED", "即時更新連線過多")));
        }
        *n += 1;
        Ok(StreamSlot { _permit: permit, uid })
    }
}

impl Drop for StreamSlot {
    fn drop(&mut self) {
        let mut g = STREAMS_PER_USER.lock().unwrap();
        if let Some(m) = g.as_mut() {
            if let Some(n) = m.get_mut(&self.uid) {
                *n -= 1;
                if *n == 0 {
                    m.remove(&self.uid);
                }
            }
        }
    }
}

/// (連線中, 自上次呼叫以來被拒絕)
pub fn stream_stats() -> (usize, u64) {
    (MAX_STREAMS_TOTAL - STREAMS_TOTAL.available_permits(), STREAMS_REJECTED.swap(0, Ordering::Relaxed))
}

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

/// 即時訊號：只送 `event: inbox`、`data: {}`，內容由 REST 取得。
/// 連線最長 25–35 分鐘（依 user id 分散，避免同時重連），客戶端重連時重新驗證 session。
async fn stream(State(st): State<AppState>, u: CurrentUser) -> Result<Response, AppError> {
    let slot = StreamSlot::acquire(u.id)?;
    let mut rx = crate::dashboard::subscribe(&st.pool).await?;
    let (tx, out) = tokio::sync::mpsc::channel::<Result<Event, std::convert::Infallible>>(16);
    tokio::spawn(async move {
        let _slot = slot;
        let secs = 25 * 60 + (u.id.as_u128() % 600) as u64;
        let deadline = tokio::time::sleep(std::time::Duration::from_secs(secs));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_slots_are_per_user_and_released_on_drop() {
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        let held: Vec<_> = (0..MAX_STREAMS_PER_USER).map(|_| StreamSlot::acquire(a).ok().unwrap()).collect();
        assert!(StreamSlot::acquire(a).is_err(), "超過每人上限");
        let other = StreamSlot::acquire(b);
        assert!(other.is_ok(), "同 IP 的其他使用者不受影響");
        drop(held);
        assert!(StreamSlot::acquire(a).is_ok(), "斷線後歸還");
    }
}
