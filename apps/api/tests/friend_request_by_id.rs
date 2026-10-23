//! Q9：好友邀請可用使用者 ID（UUID）指定對象；語意與 Email / @handle 相同（一律 202、不洩漏帳號）。
mod common;

use axum::http::StatusCode;
use common::{befriend, ck, user};
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

async fn req(pool: &PgPool, token: &str, to: &str) -> (StatusCode, Value) {
    let (s, _, b) = common::call(
        pool,
        "POST",
        "/friends/requests",
        &ck(token),
        Some(json!({ "to": to })),
    )
    .await;
    (s, b)
}

async fn pending(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM friend_requests")
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn friends(pool: &PgPool, a: Uuid, b: Uuid) -> bool {
    let (a, b) = if a < b { (a, b) } else { (b, a) };
    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM friendships WHERE user_a=$1 AND user_b=$2)")
        .bind(a)
        .bind(b)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn request_by_id_reaches_target_incoming(pool: PgPool) {
    let a = user(&pool, "A").await;
    let b = user(&pool, "B").await; // 沒有 handle
    let (s, body) = req(&pool, &a.token, &b.id.to_string()).await;
    assert_eq!(s, StatusCode::ACCEPTED);
    let (_, _, list) = common::call(&pool, "GET", "/friends", &ck(&b.token), None).await;
    let incoming = list["incoming"].as_array().unwrap();
    assert_eq!(incoming.len(), 1, "{list}");
    assert_eq!(incoming[0]["user"]["id"], json!(a.id), "{list}");

    // 不存在的 UUID 與存在的帳號回應完全相同，且不寫入任何資料
    let (s2, body2) = req(&pool, &a.token, &Uuid::new_v4().to_string()).await;
    assert_eq!((s2, body2), (s, body));
    assert_eq!(pending(&pool).await, 1);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn request_by_id_self_friend_deleted_are_silent(pool: PgPool) {
    let a = user(&pool, "A").await;
    let f = user(&pool, "F").await;
    let gone = user(&pool, "Gone").await;
    befriend(&pool, a.id, f.id).await;
    sqlx::query("UPDATE users SET deleted_at = now() WHERE id = $1")
        .bind(gone.id)
        .execute(&pool)
        .await
        .unwrap();
    for to in [a.id, f.id, gone.id] {
        let (s, body) = req(&pool, &a.token, &to.to_string()).await;
        assert_eq!((s, body), (StatusCode::ACCEPTED, json!({})));
    }
    assert_eq!(pending(&pool).await, 0);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn mutual_request_by_id_auto_befriends(pool: PgPool) {
    let a = user(&pool, "A").await;
    let b = user(&pool, "B").await;
    req(&pool, &a.token, &b.id.to_string()).await;
    assert!(!friends(&pool, a.id, b.id).await);
    // 大寫 UUID 也要能解析
    req(&pool, &b.token, &a.id.to_string().to_uppercase()).await;
    assert!(friends(&pool, a.id, b.id).await);
    assert_eq!(pending(&pool).await, 0);
}
