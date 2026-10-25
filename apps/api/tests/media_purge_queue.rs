//! 圖片刪除待辦存在 DB：程序重啟後 drain 接著做完，R2 暫時故障也會在下一輪補刪。
mod common;
use axum::{extract::Request, http::StatusCode, routing::delete, Router};
use common::*;
use sqlx::PgPool;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use uuid::Uuid;
use wishly_api::config::Config;

#[sqlx::test(migrations = "../../db/migrations")]
async fn drain_finishes_requests_left_by_a_restart(pool: PgPool) {
    let deleted = Arc::new(Mutex::new(Vec::<String>::new()));
    let fail_once = Arc::new(AtomicBool::new(true));
    let (seen, fail) = (deleted.clone(), fail_once.clone());
    let mock = Router::new().route(
        "/media/{*key}",
        delete(move |request: Request| {
            let (seen, fail) = (seen.clone(), fail.clone());
            async move {
                if fail.swap(false, Ordering::SeqCst) {
                    return StatusCode::INTERNAL_SERVER_ERROR;
                }
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
    let list = wishlist(&pool, owner.id, ListOpts::default()).await;
    let key = |dir: &str| format!("{dir}/{}.jpg", Uuid::new_v4());
    let (requested, retired, shared) = (key("items"), key("covers"), key("covers"));
    // 重啟前留下的狀態：已要求但未退役、已退役但 origin 未刪、要求了但仍被引用
    sqlx::query(
        "INSERT INTO user_media (user_id, object_key, retire_requested_at, deleted_at) VALUES
           ($1, $2, now(), NULL), ($1, $3, now(), now()), ($1, $4, now(), NULL)",
    )
    .bind(owner.id)
    .bind(&requested)
    .bind(&retired)
    .bind(&shared)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("UPDATE wishlists SET cover_image_key=$2 WHERE id=$1")
        .bind(list.id)
        .bind(&shared)
        .execute(&pool)
        .await
        .unwrap();

    assert!(
        !wishly_api::uploads::drain_media(&pool).await,
        "R2 失敗時須回報未完成"
    );
    assert!(
        wishly_api::uploads::drain_media(&pool).await,
        "下一輪須補刪完成"
    );
    let mut got = deleted.lock().unwrap().clone();
    got.sort();
    let mut want = vec![requested.clone(), retired.clone()];
    want.sort();
    assert_eq!(got, want, "被引用的圖片不可刪");
    let pending: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM user_media WHERE (retire_requested_at IS NOT NULL AND deleted_at IS NULL) OR (deleted_at IS NOT NULL AND purged_at IS NULL)",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(pending, 0, "待辦須清空（被引用者撤銷要求）");
    let shared_retired: bool =
        sqlx::query_scalar("SELECT deleted_at IS NOT NULL FROM user_media WHERE object_key=$1")
            .bind(&shared)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(!shared_retired);
    server.abort();
}
