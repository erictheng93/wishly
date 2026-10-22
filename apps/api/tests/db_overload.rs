//! 6.4 降級：資料庫壅塞回 503 SERVICE_UNAVAILABLE + Retry-After，而不是 500（流程驗證 #29 稽核）。
mod common;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::time::Duration;

#[sqlx::test(migrations = "../../db/migrations")]
async fn pool_exhausted_returns_503_with_retry_after(pool: PgPool) {
    // 只有一條連線且被占住：下一個請求在 acquire_timeout 後失敗
    let tiny = PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_millis(300))
        .connect_with((*pool.connect_options()).clone())
        .await
        .unwrap();
    let _held = tiny.acquire().await.unwrap();
    let app = wishly_api::app(wishly_api::AppState { pool: tiny.clone() });
    let (s, h, b) = common::call_app(&app, "GET", "/public/wishlists?page=1", &[], None).await;
    assert_eq!(
        (s, b["code"].as_str()),
        (StatusCode::SERVICE_UNAVAILABLE, Some("SERVICE_UNAVAILABLE")),
        "{b}"
    );
    assert_eq!(
        h.get("retry-after").and_then(|v| v.to_str().ok()),
        Some("2")
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn lock_timeout_maps_to_503(pool: PgPool) {
    let mut a = pool.begin().await.unwrap();
    sqlx::raw_sql("CREATE TABLE t (id int PRIMARY KEY); INSERT INTO t VALUES (1)")
        .execute(&mut *a)
        .await
        .unwrap();
    a.commit().await.unwrap();
    let mut holder = pool.begin().await.unwrap();
    sqlx::query("SELECT 1 FROM t WHERE id = 1 FOR UPDATE")
        .execute(&mut *holder)
        .await
        .unwrap();
    let mut waiter = pool.begin().await.unwrap();
    sqlx::query("SET LOCAL lock_timeout = '100ms'")
        .execute(&mut *waiter)
        .await
        .unwrap();
    let e = sqlx::query("SELECT 1 FROM t WHERE id = 1 FOR UPDATE")
        .execute(&mut *waiter)
        .await
        .unwrap_err();
    let res = wishly_api::error::AppError::Db(e).into_response();
    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
    // 其他資料庫錯誤維持 500
    let other = wishly_api::error::AppError::Db(sqlx::Error::RowNotFound).into_response();
    assert_eq!(other.status(), StatusCode::INTERNAL_SERVER_ERROR);
}
