//! 封鎖與檢舉使用者（0060）。封鎖對雙方對稱；檢舉進既有 content_reports 佇列。
use crate::{error::AppError, friends::lock_pair, ratelimit, session::CurrentUser, AppState};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/users/{id}/block", post(block).delete(unblock))
        .route("/users/{id}/reports", post(report))
        .route("/me/blocks", get(list))
}

async fn live_user(st: &AppState, id: Uuid) -> Result<(), AppError> {
    sqlx::query_scalar::<_, Uuid>("SELECT id FROM users WHERE id = $1 AND deleted_at IS NULL")
        .bind(id)
        .fetch_optional(&st.pool)
        .await?
        .map(|_| ())
        .ok_or(AppError::NotFound)
}

fn not_self(me: Uuid, id: Uuid) -> Result<(), AppError> {
    if me == id {
        return Err(AppError::invalid("/id", "SELF", "不能對自己操作"));
    }
    Ok(())
}

async fn block(
    u: CurrentUser,
    State(st): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    not_self(u.id, id)?;
    ratelimit::check_pool(&st.pool, &format!("user_block:{}", u.id), 60, 3600).await?;
    live_user(&st, id).await?;
    let mut tx = st.pool.begin().await?;
    lock_pair(&mut tx, u.id, id).await?; // 鎖序：pair lock 一律最先
    sqlx::query(
        "INSERT INTO user_blocks (blocker_id, blocked_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
    )
    .bind(u.id)
    .bind(id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM friendships WHERE user_a = LEAST($1::uuid, $2::uuid) AND user_b = GREATEST($1::uuid, $2::uuid)")
        .bind(u.id)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM friend_requests WHERE (from_user = $1 AND to_user = $2) OR (from_user = $2 AND to_user = $1)")
        .bind(u.id)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "DELETE FROM close_friends WHERE (owner_id = $1 AND friend_id = $2) OR (owner_id = $2 AND friend_id = $1)",
    )
    .bind(u.id)
    .bind(id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn unblock(
    u: CurrentUser,
    State(st): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    sqlx::query("DELETE FROM user_blocks WHERE blocker_id = $1 AND blocked_id = $2")
        .bind(u.id)
        .bind(id)
        .execute(&st.pool)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list(u: CurrentUser, State(st): State<AppState>) -> Result<Json<Value>, AppError> {
    #[allow(clippy::type_complexity)]
    let rows: Vec<(
        Uuid,
        Option<String>,
        String,
        Option<String>,
        chrono::DateTime<chrono::Utc>,
    )> = sqlx::query_as(
        "SELECT o.id, o.handle, o.display_name, o.avatar_key, b.created_at FROM user_blocks b
               JOIN users o ON o.id = b.blocked_id AND o.deleted_at IS NULL
              WHERE b.blocker_id = $1 ORDER BY b.created_at DESC, o.id",
    )
    .bind(u.id)
    .fetch_all(&st.pool)
    .await?;
    let users: Vec<Value> = rows
        .into_iter()
        .map(|(id, h, n, a, at)| {
            json!({ "id": id, "handle": h, "display_name": n,
                    "avatar_url": crate::uploads::avatar_url(a.as_deref()), "blocked_at": at })
        })
        .collect();
    Ok(Json(json!({ "users": users })))
}

#[derive(Deserialize)]
struct NewReport {
    reason: String,
    detail: Option<String>,
}

async fn report(
    u: CurrentUser,
    State(st): State<AppState>,
    Path(id): Path<Uuid>,
    Json(b): Json<NewReport>,
) -> Result<(StatusCode, Json<Value>), AppError> {
    if ![
        "scam",
        "inappropriate",
        "copyright",
        "personal_info",
        "other",
    ]
    .contains(&b.reason.as_str())
    {
        return Err(AppError::invalid("/reason", "INVALID", "reason 不合法"));
    }
    not_self(u.id, id)?;
    let detail = match b.detail.as_deref() {
        Some(d) => Some(crate::validate::text(d, "/detail", true)?).filter(|d| !d.is_empty()),
        None => None,
    };
    if detail.as_deref().is_some_and(|d| d.chars().count() > 1000) {
        return Err(AppError::invalid("/detail", "TOO_LONG", "最多 1000 字"));
    }
    live_user(&st, id).await?;
    let dup: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM content_reports WHERE reported_user_id = $1 AND reporter_user_id = $2
            AND created_at > now() - interval '24 hours' LIMIT 1",
    )
    .bind(id)
    .bind(u.id)
    .fetch_optional(&st.pool)
    .await?;
    if let Some(id) = dup {
        return Ok((
            StatusCode::CREATED,
            Json(json!({ "id": id, "status": "open" })),
        ));
    }
    ratelimit::check_pool(&st.pool, &format!("user_report:{}", u.id), 10, 3600).await?;
    let rid: Uuid = sqlx::query_scalar(
        "INSERT INTO content_reports (reported_user_id, reason, detail, reporter_user_id, snapshot)
         SELECT $1, $2::report_reason, $3, $4,
           jsonb_build_object('display_name', display_name, 'handle', handle, 'bio', bio, 'avatar_key', avatar_key)
         FROM users WHERE id = $1 RETURNING id",
    )
    .bind(id)
    .bind(&b.reason)
    .bind(&detail)
    .bind(u.id)
    .fetch_one(&st.pool)
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "id": rid, "status": "open" })),
    ))
}
