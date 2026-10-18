//! DELETE /me 後清掉使用者自己的 R2 物件；仍被他人引用的 key 不可刪。
mod common;
use axum::{extract::Request, http::StatusCode, routing::delete, Router};
use common::*;
use serde_json::json;
use sqlx::PgPool;
use std::time::Duration;
use tokio::sync::mpsc;
use uuid::Uuid;
use wishly_api::config::Config;

#[sqlx::test(migrations = "../../db/migrations")]
async fn delete_me_removes_unshared_media_only(pool: PgPool) {
    let (seen, mut received) = mpsc::unbounded_channel();
    let mock = Router::new().route(
        "/media/{*key}",
        delete(move |request: Request| {
            let seen = seen.clone();
            async move {
                seen.send(request.uri().path().to_string()).unwrap();
                StatusCode::NO_CONTENT
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut cfg = Config::dev();
    cfg.s3.endpoint = format!("http://{}", listener.local_addr().unwrap());
    cfg.s3.bucket = "media".into();
    wishly_api::config::init(cfg);
    let server = tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });

    let TestUser { id: uid, token } = user(&pool, "小米").await;
    let TestUser { id: other, .. } = user(&pool, "o").await;
    let k = |dir: &str| format!("{dir}/{}.jpg", Uuid::new_v4());
    let (cover, item_own, shared, avatar) = (k("covers"), k("items"), k("items"), k("avatars"));
    let mine = wishlist(&pool, uid, ListOpts::default()).await;
    let theirs = wishlist(&pool, other, ListOpts::default()).await;
    sqlx::query("UPDATE wishlists SET cover_image_key=$2, cover_image_status='ready' WHERE id=$1")
        .bind(mine.id)
        .bind(&cover)
        .execute(&pool)
        .await
        .unwrap();
    for (wl, key) in [
        (mine.id, &item_own),
        (mine.id, &shared),
        (theirs.id, &shared),
    ] {
        let it = item(&pool, wl, "x", 1).await;
        sqlx::query("UPDATE wishlist_items SET image_key=$2, image_status='ready' WHERE id=$1")
            .bind(it)
            .bind(key)
            .execute(&pool)
            .await
            .unwrap();
    }
    sqlx::query("UPDATE users SET avatar_key=$2 WHERE id=$1")
        .bind(uid)
        .bind(&avatar)
        .execute(&pool)
        .await
        .unwrap();

    let (s, _, _) = call(
        &pool,
        "DELETE",
        "/me",
        &ck(&token),
        Some(json!({"confirm":"DELETE"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    let mut got = Vec::new();
    for _ in 0..3 {
        let p = tokio::time::timeout(Duration::from_secs(5), received.recv())
            .await
            .expect("應收到 3 個 DELETE")
            .unwrap();
        got.push(p);
    }
    got.sort();
    let mut want: Vec<String> = [&cover, &item_own, &avatar]
        .iter()
        .map(|k| format!("/media/{k}"))
        .collect();
    want.sort();
    assert_eq!(got, want);
    // 他人仍引用的 key 不可被刪
    assert!(
        tokio::time::timeout(Duration::from_millis(500), received.recv())
            .await
            .is_err()
    );
    server.abort();
}
