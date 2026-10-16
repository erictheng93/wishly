mod common;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use sqlx::PgPool;
use tower::ServiceExt;

use common::*;

// dashboard::BUS 是 process-wide OnceCell，只監聽第一次成功初始化的 pool，關閉後也不會重綁。
// 此 integration-test binary 的所有 SSE 測試必須共用同一 pool；勿新增使用另一個 sqlx::test pool 的 SSE 測試。
#[sqlx::test(migrations = "../../db/migrations")]
async fn sse_pushes_item_update_and_410_when_hidden(pool: PgPool) {
    let TestUser {
        id: owner,
        token: _,
    } = user(&pool, "o").await;
    let (wid, slug, item) = claimed_wishlist(&pool, owner, false, 30, 2, 1).await;
    let req = Request::builder()
        .uri(format!("/api/v1/public/wishlists/{slug}/events"))
        .body(Body::empty())
        .unwrap();
    let res = test_app(&pool).oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers()["content-type"], "text/event-stream");
    let mut body = res.into_body();
    sqlx::query("UPDATE wishlist_items SET qty_claimed = 2 WHERE id = $1")
        .bind(item)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE claims SET qty=2 WHERE item_id=$1")
        .bind(item)
        .execute(&pool)
        .await
        .unwrap();
    wishly_api::dashboard::notify(&pool, wid).await.unwrap();
    let mut got = String::new();
    while !got.contains("wishlist.updated") {
        let f = tokio::time::timeout(std::time::Duration::from_secs(5), body.frame())
            .await
            .expect("sse timeout")
            .unwrap()
            .unwrap();
        got.push_str(&String::from_utf8_lossy(f.data_ref().unwrap()));
    }
    assert!(
        got.contains("event: item.updated")
            && got.contains("\"qty_claimed\":2")
            && got.contains("\"is_fully_claimed\":true"),
        "{got}"
    );
    assert!(!got.contains("秘密客"));
    // 下架 → 新連線 410
    sqlx::query(
        "UPDATE wishlists SET moderation_status='hidden', moderation_reason='x' WHERE id=$1",
    )
    .bind(wid)
    .execute(&pool)
    .await
    .unwrap();
    let req = Request::builder()
        .uri(format!("/api/v1/public/wishlists/{slug}/events"))
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        test_app(&pool).oneshot(req).await.unwrap().status(),
        StatusCode::GONE
    );
    assert_all_reconciled(&pool).await;
    drop(body);
    tokio::time::timeout(std::time::Duration::from_secs(10), pool.close())
        .await
        .expect("SSE listener must release its connection when the pool closes");
}
