//! 帳號摯友名單（0067）：擁有者自己的好友子集合，visibility=selected 的清單只給名單內的人看。
//! 單向存放（A 把 B 當摯友不代表反向）；解除好友 / 封鎖時由 friends.rs / blocks.rs 雙向清掉。
use crate::{error::AppError, friends::lock_pair, session::CurrentUser, AppState};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, put},
    Json, Router,
};
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

const MAX_CLOSE: i64 = 200;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/me/close-friends", get(list))
        .route("/me/close-friends/{user_id}", put(add).delete(remove))
}

/// `viewer` 是否在 `owner` 的摯友名單內
pub async fn is_close(pool: &PgPool, owner: Uuid, viewer: Uuid) -> Result<bool, AppError> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM close_friends c WHERE c.owner_id = $1 AND c.friend_id = $2)",
    )
    .bind(owner)
    .bind(viewer)
    .fetch_one(pool)
    .await?)
}

async fn list(u: CurrentUser, State(st): State<AppState>) -> Result<Json<Value>, AppError> {
    let rows: Vec<(Uuid, String, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT x.id, x.display_name, x.handle, x.avatar_key FROM close_friends c JOIN users x ON x.id = c.friend_id
          WHERE c.owner_id = $1 AND x.deleted_at IS NULL ORDER BY c.created_at, x.id",
    )
    .bind(u.id)
    .fetch_all(&st.pool)
    .await?;
    let users: Vec<Value> = rows
        .into_iter()
        .map(|(id, n, h, a)| {
            json!({ "id": id, "display_name": n, "handle": h, "avatar_url": crate::uploads::avatar_url(a.as_deref()) })
        })
        .collect();
    Ok(Json(json!({ "users": users })))
}

async fn add(
    u: CurrentUser,
    State(st): State<AppState>,
    Path(other): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    if other == u.id {
        return Err(AppError::problem(422, "SELF", "不能把自己加入摯友"));
    }
    let mut tx = st.pool.begin().await?;
    lock_pair(&mut tx, u.id, other).await?; // 鎖序：pair lock 一律最先
    let friends: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM friendships WHERE user_a = LEAST($1::uuid, $2::uuid) AND user_b = GREATEST($1::uuid, $2::uuid))",
    )
    .bind(u.id)
    .bind(other)
    .fetch_one(&mut *tx)
    .await?;
    if !friends {
        return Err(AppError::problem(
            422,
            "NOT_FRIEND",
            "摯友只能從你的好友中選擇",
        ));
    }
    // 已在名單內的重複 PUT 不受上限影響（冪等）；shortcut: 上限檢查未跨 pair 鎖，併發最多略超 200
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM close_friends WHERE owner_id = $1")
        .bind(u.id)
        .fetch_one(&mut *tx)
        .await?;
    let r = sqlx::query(
        "INSERT INTO close_friends (owner_id, friend_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
    )
    .bind(u.id)
    .bind(other)
    .execute(&mut *tx)
    .await?;
    if r.rows_affected() == 1 {
        if n >= MAX_CLOSE {
            return Err(AppError::problem(422, "LIMIT", "摯友最多 200 位"));
        }
        crate::security::audit(
            &mut *tx,
            u.id,
            "close_friends.add",
            "users",
            other,
            None,
            json!({}),
        )
        .await?;
    }
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn remove(
    u: CurrentUser,
    State(st): State<AppState>,
    Path(other): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    let mut tx = st.pool.begin().await?;
    lock_pair(&mut tx, u.id, other).await?;
    let r = sqlx::query("DELETE FROM close_friends WHERE owner_id = $1 AND friend_id = $2")
        .bind(u.id)
        .bind(other)
        .execute(&mut *tx)
        .await?;
    if r.rows_affected() == 1 {
        crate::security::audit(
            &mut *tx,
            u.id,
            "close_friends.remove",
            "users",
            other,
            None,
            json!({}),
        )
        .await?;
    }
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
