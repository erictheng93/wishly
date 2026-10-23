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

/// 占住品項列鎖，回傳持鎖交易（drop 即釋放）
async fn hold_item(pool: &PgPool, item: uuid::Uuid) -> sqlx::Transaction<'static, sqlx::Postgres> {
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT 1 FROM wishlist_items WHERE id = $1 FOR UPDATE")
        .bind(item)
        .execute(&mut *tx)
        .await
        .unwrap();
    tx
}

/// 呼叫須在 lock_timeout（2 秒）後回 503，而不是一直排隊；10 秒內沒回就是沒設逾時
async fn expect_503_after_lock_wait(
    app: &axum::Router,
    method: &str,
    uri: &str,
    h: &common::Hdr,
    body: serde_json::Value,
) {
    let t = std::time::Instant::now();
    let (s, _, b) = tokio::time::timeout(
        Duration::from_secs(10),
        common::call_app(app, method, uri, h, Some(body)),
    )
    .await
    .expect("請求一直卡在等鎖：交易沒有設 lock_timeout");
    assert_eq!(
        (s, b["code"].as_str()),
        (StatusCode::SERVICE_UNAVAILABLE, Some("SERVICE_UNAVAILABLE")),
        "{b}"
    );
    assert!(
        t.elapsed() >= Duration::from_millis(1500),
        "{:?}",
        t.elapsed()
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn claim_waiting_on_hot_item_gives_up_with_503(pool: PgPool) {
    let owner = common::user(&pool, "owner").await;
    let donor = common::user(&pool, "donor").await;
    let wl = common::wishlist(&pool, owner.id, common::ListOpts::default())
        .await
        .id;
    let it = common::item(&pool, wl, "熱門品項", 5).await;
    let app = common::test_app(&pool);
    let held = hold_item(&pool, it).await;
    let h = [common::ck(&donor.token), common::idem(uuid::Uuid::new_v4())].concat();
    expect_503_after_lock_wait(
        &app,
        "POST",
        &format!("/items/{it}/claims"),
        &h,
        serde_json::json!({"qty":1}),
    )
    .await;
    drop(held);
    // 逾時的交易已回滾、idempotency key 沒被占用：鎖釋放後同 key 重試成功
    let (s, _, b) = common::call_app(
        &app,
        "POST",
        &format!("/items/{it}/claims"),
        &h,
        Some(serde_json::json!({"qty":1})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    common::assert_claims_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn pledge_waiting_on_hot_item_gives_up_with_503(pool: PgPool) {
    let owner = common::user(&pool, "owner").await;
    let donor = common::user(&pool, "donor").await;
    let wl = common::wishlist(&pool, owner.id, common::ListOpts::default())
        .await
        .id;
    sqlx::query("INSERT INTO shipping_addresses(wishlist_id,recipient_name_enc,phone_enc,address_enc) VALUES($1,$2,$2,$2)")
        .bind(wl).bind(wishly_api::sealed::seal(b"test address")).execute(&pool).await.unwrap();
    let it = common::crowdfund_item(&pool, wl, 1000).await;
    common::grant(&pool, donor.id, 100).await;
    let app = common::test_app(&pool);
    let held = hold_item(&pool, it).await;
    let h = [common::ck(&donor.token), common::idem(uuid::Uuid::new_v4())].concat();
    expect_503_after_lock_wait(
        &app,
        "POST",
        &format!("/items/{it}/contributions"),
        &h,
        serde_json::json!({"points":50}),
    )
    .await;
    drop(held);
    assert_eq!(common::balance(&pool, donor.id).await, 100);
    common::assert_points_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn claim_status_change_waiting_on_hot_item_gives_up_with_503(pool: PgPool) {
    let owner = common::user(&pool, "owner").await;
    let wl = common::wishlist(&pool, owner.id, common::ListOpts::default())
        .await
        .id;
    let it = common::item(&pool, wl, "熱門品項", 5).await;
    let (token, claim) = common::user_claim(&pool, it, 1).await;
    let app = common::test_app(&pool);
    let held = hold_item(&pool, it).await;
    let h = [common::ck(&token), common::idem(uuid::Uuid::new_v4())].concat();
    expect_503_after_lock_wait(
        &app,
        "PATCH",
        &format!("/claims/{claim}"),
        &h,
        serde_json::json!({"status":"purchased"}),
    )
    .await;
    drop(held);
    let (s, _, b) = common::call_app(
        &app,
        "PATCH",
        &format!("/claims/{claim}"),
        &h,
        Some(serde_json::json!({"status":"purchased"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
}
