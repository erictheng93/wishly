//! 資料匯出、確認信與退訂。
mod common;
use axum::http::StatusCode;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;
use wishly_api::session::hash_token;

use common::*;

#[sqlx::test(migrations = "../../db/migrations")]
async fn no_confirmation_without_email(pool: PgPool) {
    let TestUser { id: o, token: _ } = user(&pool, "u").await;
    let wl = wishlist(
        &pool,
        o,
        ListOpts {
            surprise: false,
            event_offset_days: Some(0),
            ..Default::default()
        },
    )
    .await;
    let it = item(&pool, wl.id, "奶瓶", 3).await;
    user_claim(&pool, it, 1).await;
    let n: i64 =
        sqlx::query_scalar("SELECT count(*) FROM notifications WHERE kind='claim.confirmation'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(n, 0);
    assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn export_and_unsubscribe(pool: PgPool) {
    let TestUser { id: u, token: t } = user(&pool, "u").await;
    let wl = wishlist(
        &pool,
        u,
        ListOpts {
            surprise: false,
            event_offset_days: Some(0),
            ..Default::default()
        },
    )
    .await;
    let it = item(&pool, wl.id, "奶瓶", 3).await;
    user_claim(&pool, it, 1).await;
    let (s, h, v) = call(&pool, "GET", "/me/export", &ck(&t), None).await;
    assert_eq!(s, StatusCode::OK);
    assert!(h["content-disposition"]
        .to_str()
        .unwrap()
        .contains("wishly-export-"));
    assert_eq!(
        (
            v["wishlists"][0]["items"][0]["title"].as_str(),
            v["claims"].as_array().map(Vec::len)
        ),
        (Some("奶瓶"), Some(0))
    );
    assert_eq!(
        call(&pool, "GET", "/me/export", &[], None).await.0,
        StatusCode::UNAUTHORIZED
    );
    call(&pool, "GET", "/me/export", &ck(&t), None).await;
    call(&pool, "GET", "/me/export", &ck(&t), None).await;
    assert_eq!(
        call(&pool, "GET", "/me/export", &ck(&t), None).await.0,
        StatusCode::TOO_MANY_REQUESTS
    );

    // 退訂：建立者。GET 只轉址、不變更狀態；POST 才退訂
    sqlx::query(
        "INSERT INTO notifications (user_id, channel, kind) VALUES ($1,'email','claim.digest')",
    )
    .bind(u)
    .execute(&pool)
    .await
    .unwrap();
    let tok = wishly_api::account::unsub_token('u', u);
    let before: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM notifications WHERE user_id=$1 AND status='pending'",
    )
    .bind(u)
    .fetch_one(&pool)
    .await
    .unwrap();
    let (s, h, _) = call(
        &pool,
        "GET",
        &format!("/unsubscribe?token={tok}"),
        &[],
        None,
    )
    .await;
    assert_eq!(s, StatusCode::FOUND);
    assert!(h["location"]
        .to_str()
        .unwrap()
        .ends_with(&format!("/unsubscribe?token={tok}")));
    let pend: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM notifications WHERE user_id=$1 AND status='pending'",
    )
    .bind(u)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(pend, before, "GET 不得變更狀態");
    for _ in 0..2 {
        assert_eq!(
            call(
                &pool,
                "POST",
                "/unsubscribe",
                &[],
                Some(json!({ "token": tok }))
            )
            .await
            .0,
            StatusCode::NO_CONTENT
        );
    }
    let off: bool = sqlx::query_scalar(
        "SELECT (notification_prefs->>'email_claims')::boolean = false FROM users WHERE id=$1",
    )
    .bind(u)
    .fetch_one(&pool)
    .await
    .unwrap();
    let pend: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM notifications WHERE user_id=$1 AND status='pending'",
    )
    .bind(u)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(off && pend == 0);
    // 訪客：清 email
    let g: Uuid = sqlx::query_scalar("INSERT INTO guests (guest_token_hash, display_name, email) VALUES ($1,'g','g@example.com') RETURNING id").bind(hash_token("g")).fetch_one(&pool).await.unwrap();
    let gtok = wishly_api::account::unsub_token('g', g);
    call(
        &pool,
        "GET",
        &format!("/unsubscribe?token={gtok}"),
        &[],
        None,
    )
    .await;
    assert!(
        sqlx::query_scalar::<_, Option<String>>("SELECT email FROM guests WHERE id=$1")
            .bind(g)
            .fetch_one(&pool)
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(
        call(
            &pool,
            "POST",
            "/unsubscribe",
            &[],
            Some(json!({ "token": gtok }))
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    let em: Option<String> = sqlx::query_scalar("SELECT email FROM guests WHERE id=$1")
        .bind(g)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(em.is_none());
    // 無效 / 竄改 → POST 422 INVALID_TOKEN；GET 仍只轉址
    for bad in [format!("{tok}x"), "garbage".into(), String::new()] {
        let (s, _, v) = call(
            &pool,
            "POST",
            "/unsubscribe",
            &[],
            Some(json!({ "token": bad })),
        )
        .await;
        assert!(s == StatusCode::UNPROCESSABLE_ENTITY && v["code"] == "INVALID_TOKEN");
        assert_eq!(
            call(
                &pool,
                "GET",
                &format!("/unsubscribe?token={bad}"),
                &[],
                None
            )
            .await
            .0,
            StatusCode::FOUND
        );
    }
    assert_eq!(
        call(&pool, "POST", "/unsubscribe", &[], Some(json!({})))
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_all_reconciled(&pool).await;
}
