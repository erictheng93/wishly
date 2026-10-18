//! 共用 fixture 的選項、輸送介面與捐助狀態定義。
mod common;
use axum::{http::HeaderMap, routing::post, Json, Router};
use common::*;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

#[sqlx::test(migrations = "../../db/migrations")]
async fn fixtures_preserve_options_quantities_and_reconcile_domain_state(pool: PgPool) {
    let owner = user(&pool, "owner").await;
    let operator = staff(&pool, "staff").await;
    befriend(&pool, operator.id, owner.id).await;
    let ordered: bool = sqlx::query_scalar("SELECT user_a < user_b FROM friendships")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(ordered);
    assert!(
        sqlx::query_scalar::<_, bool>("SELECT is_staff FROM users WHERE id=$1")
            .bind(operator.id)
            .fetch_one(&pool)
            .await
            .unwrap()
    );
    assert_eq!(
        call(&pool, "GET", "/me", &ck(&owner.token), None).await.0,
        axum::http::StatusCode::OK
    );
    let wl = wishlist(&pool, owner.id, ListOpts::default()).await;
    assert_eq!(wl.slug.len(), 10);
    let defaults: (String,String,bool,bool,bool,String) = sqlx::query_as("SELECT status::text,visibility::text,surprise_mode,event_date IS NULL,closes_at IS NULL,moderation_status::text FROM wishlists WHERE id=$1").bind(wl.id).fetch_one(&pool).await.unwrap();
    assert_eq!(
        defaults,
        (
            "active".into(),
            "public".into(),
            false,
            true,
            true,
            "ok".into()
        )
    );
    let hidden = wishlist(
        &pool,
        owner.id,
        ListOpts {
            status: Some("archived"),
            visibility: Some("private"),
            surprise: true,
            event_offset_days: Some(30),
            closes_in: Some("1 hour"),
            moderation_status: Some("hidden"),
        },
    )
    .await;
    let opts: (String,String,bool,bool,bool,String,bool) = sqlx::query_as("SELECT status::text,visibility::text,surprise_mode,event_date=current_date+30,closes_at-created_at=interval '1 hour',moderation_status::text,moderation_reason IS NOT NULL FROM wishlists WHERE id=$1").bind(hidden.id).fetch_one(&pool).await.unwrap();
    assert_eq!(
        opts,
        (
            "archived".into(),
            "private".into(),
            true,
            true,
            true,
            "hidden".into(),
            true
        )
    );
    let it = item(&pool, wl.id, "quantity", 5).await;
    let (_, claim) = user_claim(&pool, it, 3).await;
    for status in ["reserved", "purchased", "delivered"] {
        sqlx::query("UPDATE claims SET status=$2::claim_status WHERE id=$1")
            .bind(claim)
            .bind(status)
            .execute(&pool)
            .await
            .unwrap();
        assert_all_reconciled(&pool).await;
    }
    // 對帳必須能拒絕快取漂移，避免測試只涵蓋綠燈。
    sqlx::query("UPDATE wishlist_items SET qty_claimed=4 WHERE id=$1")
        .bind(it)
        .execute(&pool)
        .await
        .unwrap();
    let check = pool.clone();
    assert!(
        tokio::spawn(async move { assert_claims_reconciled(&check).await })
            .await
            .expect_err("claims reconciliation accepted a drifted cache")
            .is_panic()
    );
    sqlx::query("UPDATE wishlist_items SET qty_claimed=0 WHERE id=$1")
        .bind(it)
        .execute(&pool)
        .await
        .unwrap();
    for status in ["cancelled", "expired"] {
        sqlx::query("UPDATE claims SET status=$2::claim_status WHERE id=$1")
            .bind(claim)
            .bind(status)
            .execute(&pool)
            .await
            .unwrap();
        assert_all_reconciled(&pool).await;
    }
    grant(&pool, operator.id, 100).await;
    let cf = crowdfund_item(&pool, wl.id, 1000).await;
    let result = pledge(&test_app(&pool), cf, &operator.token, 30).await;
    assert_eq!(result.0, axum::http::StatusCode::CREATED, "{}", result.2);
    assert_eq!(balance(&pool, operator.id).await, 70);
    sqlx::query("UPDATE point_wallets SET balance=71 WHERE user_id=$1")
        .bind(operator.id)
        .execute(&pool)
        .await
        .unwrap();
    let check = pool.clone();
    assert!(
        tokio::spawn(async move { assert_points_reconciled(&check).await })
            .await
            .expect_err("wallet reconciliation accepted a drifted balance")
            .is_panic()
    );
    sqlx::query("UPDATE point_wallets SET balance=70 WHERE user_id=$1")
        .bind(operator.id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE wishlist_items SET pledged_points=31 WHERE id=$1")
        .bind(cf)
        .execute(&pool)
        .await
        .unwrap();
    let check = pool.clone();
    assert!(
        tokio::spawn(async move { assert_points_reconciled(&check).await })
            .await
            .expect_err("contribution reconciliation accepted a drifted cache")
            .is_panic()
    );
    sqlx::query("UPDATE wishlist_items SET pledged_points=30 WHERE id=$1")
        .bind(cf)
        .execute(&pool)
        .await
        .unwrap();

    assert_all_reconciled(&pool).await;
}

#[tokio::test]
async fn call_app_sets_content_type_only_for_json_and_accepts_plain_responses() {
    let app = Router::new().route("/api/v1/headers",post(|headers: HeaderMap| async move {
        Json(json!({"content_type":headers.get("content-type").map(|v|v.to_str().unwrap()),"key":headers.get("idempotency-key").map(|v|v.to_str().unwrap())}))
    })).route("/api/v1/text",post(||async {"plain"})).route("/api/v1/empty",post(||async {axum::http::StatusCode::NO_CONTENT}));
    let key = Uuid::new_v4();
    let with = call_app(&app, "POST", "/headers", &idem(key), Some(json!({}))).await;
    assert_eq!(
        with.2,
        json!({"content_type":"application/json","key":key.to_string()})
    );
    let without = call_app(&app, "POST", "/headers", &[], None).await;
    assert_eq!(without.2, json!({"content_type":null,"key":null}));
    assert!(call_app(&app, "POST", "/text", &[], None).await.2.is_null());
    let empty = call_app(&app, "POST", "/empty", &[], None).await;
    assert_eq!(empty.0, axum::http::StatusCode::NO_CONTENT);
    assert!(empty.2.is_null());
}

#[tokio::test]
#[should_panic(expected = "JSON response must be valid")]
async fn malformed_json_response_cannot_silently_turn_into_null() {
    let app = Router::new().route(
        "/api/v1/bad",
        post(|| async { ([("content-type", "application/json")], "broken") }),
    );
    call_app(&app, "POST", "/bad", &[], None).await;
}
