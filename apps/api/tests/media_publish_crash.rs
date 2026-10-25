//! 發佈與複查之間程序崩潰：DB 留下的發佈圍欄讓 drain 等期限過後再刪一次才標記已刪。
mod common;
use axum::{extract::Request, http::StatusCode, routing::delete, Router};
use common::*;
use sqlx::PgPool;
use std::sync::{Arc, Mutex};
use uuid::Uuid;
use wishly_api::config::Config;

#[sqlx::test(migrations = "../../db/migrations")]
async fn purge_waits_for_publish_fence_left_by_a_crash(pool: PgPool) {
    let deleted = Arc::new(Mutex::new(Vec::<String>::new()));
    let seen = deleted.clone();
    let mock = Router::new().route(
        "/media/{*key}",
        delete(move |request: Request| {
            let seen = seen.clone();
            async move {
                let key = request.uri().path().strip_prefix("/media/").unwrap();
                seen.lock().unwrap().push(key.to_owned());
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

    let owner = user(&pool, "owner").await;
    let key = format!("items/{}.jpg", Uuid::new_v4());
    // 崩潰前的狀態：圍欄已寫、圖片已退役，但發佈的複查沒跑到
    sqlx::query(
        "INSERT INTO user_media (user_id, object_key, deleted_at, publishing_until) VALUES ($1, $2, now(), now() + interval '10 minutes')",
    )
    .bind(owner.id)
    .bind(&key)
    .execute(&pool)
    .await
    .unwrap();
    let purged = || async {
        sqlx::query_scalar::<_, bool>(
            "SELECT purged_at IS NOT NULL FROM user_media WHERE object_key=$1",
        )
        .bind(&key)
        .fetch_one(&pool)
        .await
        .unwrap()
    };

    wishly_api::uploads::drain_media(&pool).await;
    assert_eq!(deleted.lock().unwrap().len(), 1, "圍欄期間仍要先刪一次");
    assert!(!purged().await, "圍欄未過期不可標記已刪");

    sqlx::query(
        "UPDATE user_media SET publishing_until=now() - interval '1 second' WHERE object_key=$1",
    )
    .bind(&key)
    .execute(&pool)
    .await
    .unwrap();
    assert!(wishly_api::uploads::drain_media(&pool).await);
    assert_eq!(deleted.lock().unwrap().len(), 2, "圍欄過期後須再刪一次");
    assert!(purged().await, "圍欄過期後的刪除才算刪完");
    server.abort();
}
