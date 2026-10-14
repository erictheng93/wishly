//! 訪客刪除與恢復生命週期。
mod common;
use axum::http::StatusCode;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;
use wishsync_api::session::hash_token;

use common::*;

#[sqlx::test(migrations = "../../db/migrations")]
async fn guest_delete_and_recover(pool: PgPool) {
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
    let (tok, cid) = guest_claim_with_email(&pool, it, Some("a@example.com")).await;
    // 確認信已入佇列
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM notifications WHERE kind='claim.confirmation' AND guest_id IS NOT NULL").fetch_one(&pool).await.unwrap();
    assert_eq!(n, 1);
    // 找回：一次性、換發新 token、舊 token 失效
    let gid: Uuid = sqlx::query_scalar("SELECT guest_id FROM claims WHERE id=$1")
        .bind(cid)
        .fetch_one(&pool)
        .await
        .unwrap();
    let r = "recover-token-xyz";
    sqlx::query("INSERT INTO guest_recovery_tokens (guest_id, token_hash, expires_at) VALUES ($1,$2, now() + interval '30 days')").bind(gid).bind(hash_token(r)).execute(&pool).await.unwrap();
    let (s, h, v) = call(
        &pool,
        "POST",
        "/guest/recover",
        &[],
        Some(json!({ "token": r })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(h["set-cookie"].to_str().unwrap().starts_with("ws_guest="));
    let new = v["guest_token"].as_str().unwrap().to_string();
    assert_eq!(
        call(&pool, "GET", "/guest/me", &gt(&tok), None).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(&pool, "GET", "/guest/me", &gt(&new), None).await.0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &pool,
            "POST",
            "/guest/recover",
            &[],
            Some(json!({ "token": r }))
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(&pool, "POST", "/guest/recover", &[], Some(json!({})))
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    // 刪除：claims 保留、PII 清除、待發通知取消、token 401
    assert_eq!(
        call(&pool, "DELETE", "/guest/me", &gt(&new), None).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(&pool, "GET", "/guest/me", &gt(&new), None).await.0,
        StatusCode::UNAUTHORIZED
    );
    let (name, em, dn): (String, Option<String>, bool) = sqlx::query_as(
        "SELECT display_name, email, deleted_at IS NOT NULL FROM guests WHERE id=$1",
    )
    .bind(gid)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!((name.as_str(), em, dn), ("已刪除的訪客", None, true));
    let cn: String = sqlx::query_scalar("SELECT claimer_name FROM claims WHERE id=$1")
        .bind(cid)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(cn, "已刪除的訪客");
    let pend: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM notifications WHERE guest_id=$1 AND status='pending'",
    )
    .bind(gid)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(pend, 0);
    // 已刪除訪客的權杖不可再用
    sqlx::query("INSERT INTO guest_recovery_tokens (guest_id, token_hash, expires_at) VALUES ($1,$2, now() + interval '30 days')").bind(gid).bind(hash_token("again")).execute(&pool).await.unwrap();
    assert_eq!(
        call(
            &pool,
            "POST",
            "/guest/recover",
            &[],
            Some(json!({ "token": "again" }))
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_all_reconciled(&pool).await;
}

#[test]
fn confirmation_mail_has_recovery_and_unsubscribe_links() {
    let p = json!({ "recovery_token": "TOK", "unsubscribe_url": "http://x/u?token=1" });
    let (_, body) = wishsync_api::notify::render("claim.confirmation", "", &p);
    assert!(
        body.contains("/me/claims#r=TOK") && body.contains("http://x/u?token=1"),
        "{body}"
    );
}
