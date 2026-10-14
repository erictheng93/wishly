mod common;
use axum::{
    body::Body,
    http::{Request, StatusCode},
    Router,
};
use http_body_util::BodyExt;
use sqlx::PgPool;
use tower::ServiceExt;
use wishsync_api::{app, AppState};

fn router(pool: &PgPool) -> Router {
    app(AppState { pool: pool.clone() })
}

/// 回傳 (user_id, session token)
use common::user_with_staff as user;

use common::claimed_wishlist as wishlist;

#[sqlx::test(migrations = "../../db/migrations")]
async fn sse_pushes_item_update_and_410_when_hidden(pool: PgPool) {
    let (owner, _) = user(&pool, "o", false).await;
    let (wid, slug, item) = wishlist(&pool, owner, false, 30).await;
    let req = Request::builder()
        .uri(format!("/api/v1/public/wishlists/{slug}/events"))
        .body(Body::empty())
        .unwrap();
    let res = router(&pool).oneshot(req).await.unwrap();
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
    wishsync_api::dashboard::notify(&pool, wid).await.unwrap();
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
        router(&pool).oneshot(req).await.unwrap().status(),
        StatusCode::GONE
    );
    common::assert_claims_reconciled(&pool).await;
    drop(body);
    tokio::time::timeout(std::time::Duration::from_secs(10), pool.close())
        .await
        .expect("SSE listener must release its connection when the pool closes");
}
