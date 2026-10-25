//! 刪除 origin 的待辦依嘗試時間輪替：一批永久失敗的 key 不能每輪都佔滿 100 筆而餓死後面的待辦。
mod common;
use axum::{extract::Request, http::StatusCode, routing::delete, Router};
use common::*;
use sqlx::PgPool;
use wishly_api::config::Config;

#[sqlx::test(migrations = "../../db/migrations")]
async fn failing_keys_do_not_starve_the_purge_queue(pool: PgPool) {
    // 模擬 R2：stuck/ 開頭的 key 一律拒絕
    let mock = Router::new().route(
        "/media/{*key}",
        delete(|request: Request| async move {
            if request.uri().path().starts_with("/media/items/stuck") {
                StatusCode::FORBIDDEN
            } else {
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
    // 100 筆永久失敗的 key 排在前面（依 key 排序），正常的 key 在第 101 筆
    sqlx::query(
        "INSERT INTO user_media (user_id, object_key, deleted_at)
         SELECT $1, 'items/stuck' || lpad(g::text, 3, '0') || '.jpg', now() FROM generate_series(1, 100) g
         UNION ALL SELECT $1, 'items/zz-ok.jpg', now()",
    )
    .bind(owner.id)
    .execute(&pool)
    .await
    .unwrap();

    for _ in 0..2 {
        assert!(
            !wishly_api::uploads::drain_media(&pool).await,
            "仍有失敗的 key"
        );
    }
    let purged: bool = sqlx::query_scalar(
        "SELECT purged_at IS NOT NULL FROM user_media WHERE object_key='items/zz-ok.jpg'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(purged, "失敗的 key 不可一直卡住後面的待辦");
    server.abort();
}
