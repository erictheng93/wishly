//! Enabled in normal cargo test; docker compose Mailpit is the SMTP integration fixture.
mod common;
use common::{user, wait_until, wishlist, ListOpts, TestList, TestUser};
use serde_json::{json, Value};
use sqlx::PgPool;
use wishly_api::config::Config;

#[sqlx::test(migrations = "../../db/migrations")]
async fn worker_sends_via_smtp_and_mailpit_receives_private_notification(pool: PgPool) {
    let mut cfg = Config::dev();
    cfg.cf_account_id = None;
    cfg.cf_email_api_token = None;
    wishly_api::config::init(cfg);
    let TestUser {
        id: owner,
        token: _,
    } = user(&pool, "Secret Owner").await;
    let TestList { id: wishlist, .. } = wishlist(&pool, owner, ListOpts::default()).await;
    sqlx::query("UPDATE wishlists SET title='SMTP integration wishlist',surprise_mode=true,event_date=current_date+30 WHERE id=$1").bind(wishlist).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO wishlist_items(wishlist_id,title) VALUES ($1,'Secret Gift')")
        .bind(wishlist)
        .execute(&pool)
        .await
        .unwrap();
    let email: String = sqlx::query_scalar("SELECT email FROM users WHERE id=$1")
        .bind(owner)
        .fetch_one(&pool)
        .await
        .unwrap();
    wishly_api::notify::enqueue_claim(&pool, wishlist)
        .await
        .unwrap();
    assert_eq!(wishly_api::notify::tick(&pool).await.unwrap(), 1);
    assert_eq!(wishly_api::notify::tick(&pool).await.unwrap(), 0);
    let state: (String, i16, bool, Option<String>) = sqlx::query_as("SELECT status::text,attempts,sent_at IS NOT NULL,last_error FROM notifications WHERE user_id=$1").bind(owner).fetch_one(&pool).await.unwrap();
    assert_eq!(state, ("sent".into(), 1, true, None));
    let client = reqwest::Client::new();
    let mut search = reqwest::Url::parse(&format!(
        "{}/api/v1/search",
        std::env::var("MAILPIT_API_BASE")
            .unwrap_or_else(|_| "http://localhost:8025".into())
            .trim_end_matches('/')
    ))
    .unwrap();
    search
        .query_pairs_mut()
        .append_pair("query", &format!("to:{email}"));
    wait_until("Mailpit has accepted the worker email", || async {
        let result: Value = client
            .get(search.clone())
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        result["messages"]
            .as_array()
            .is_some_and(|messages| !messages.is_empty())
    })
    .await;
    let result: Value = client
        .get(search)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let messages = result["messages"].as_array().unwrap();
    assert_eq!(
        messages.len(),
        1,
        "tick retried an already sent notification"
    );
    let message = &messages[0];
    assert_eq!(message["To"][0]["Address"], email);
    assert_eq!(
        message["Subject"],
        "【Wishly】《SMTP integration wishlist》有 1 件新捐助"
    );
    let base = std::env::var("MAILPIT_API_BASE").unwrap_or_else(|_| "http://localhost:8025".into());
    let body: Value = client
        .get(format!(
            "{}/api/v1/message/{}",
            base.trim_end_matches('/'),
            message["ID"].as_str().unwrap()
        ))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let text = body["Text"].as_str().unwrap();
    assert!(text.contains("有 1 件新捐助"));
    assert!(text.contains("/api/v1/unsubscribe?token="));
    assert!(!text.contains("Secret Gift") && !text.contains("Secret Owner"));
    let payload: Value = sqlx::query_scalar("SELECT payload FROM notifications WHERE user_id=$1")
        .bind(owner)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(payload, json!({"wishlist_id":wishlist,"count":1}));
    common::assert_all_reconciled(&pool).await;
}
