//! 登入成功且是沒見過的裝置時發安全通知：驗證碼登入不像重設密碼有任何信號，帳號被盜時本人要看得到。
mod common;
use axum::{
    http::header,
    routing::{get, post},
    Form, Json, Router,
};
use serde_json::json;
use sqlx::PgPool;
use std::collections::HashMap;
use wishly_api::auth::code_hash;

use common::*;

const CHROME_MAC: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) Chrome/130.0.6723.69";
const CHROME_MAC_NEWER: &str =
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) Chrome/131.0.6778.85";
const SAFARI_IPHONE: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_1 like Mac OS X) Safari/604.1";
const EMAIL: &str = "device@example.com";

async fn otp_login(pool: &PgPool, app: &Router, ua: &str) {
    sqlx::query("DELETE FROM rate_limits")
        .execute(pool)
        .await
        .unwrap();
    let hdr = [("user-agent", ua.to_string())];
    let (st, _, b) = call_app(
        app,
        "POST",
        "/auth/otp/request",
        &hdr,
        Some(json!({ "email": EMAIL })),
    )
    .await;
    assert_eq!(st, 202, "{b}");
    sqlx::query("UPDATE otp_challenges SET code_hash=$1 WHERE email=$2 AND consumed_at IS NULL")
        .bind(code_hash("123456", "dev-pepper"))
        .bind(EMAIL)
        .execute(pool)
        .await
        .unwrap();
    let (st, _, b) = call_app(
        app,
        "POST",
        "/auth/otp/verify",
        &hdr,
        Some(json!({ "email": EMAIL, "code": "123456", "challenge_id": b["challenge_id"] })),
    )
    .await;
    assert_eq!(st, 200, "{b}");
}

async fn notices(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM notifications WHERE kind='auth.new_device'")
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn only_an_unseen_device_triggers_the_notice(pool: PgPool) {
    let app = test_app(&pool);
    otp_login(&pool, &app, CHROME_MAC).await;
    assert_eq!(notices(&pool).await, 0, "開戶第一次登入不通知");

    otp_login(&pool, &app, CHROME_MAC).await;
    assert_eq!(notices(&pool).await, 0, "同一台裝置再登入不通知");

    otp_login(&pool, &app, CHROME_MAC_NEWER).await;
    assert_eq!(notices(&pool).await, 0, "瀏覽器更新版本不算新裝置");

    sqlx::query("UPDATE sessions SET revoked_at=now()")
        .execute(&pool)
        .await
        .unwrap();
    otp_login(&pool, &app, CHROME_MAC).await;
    assert_eq!(notices(&pool).await, 0, "登出過的裝置仍是本人用過的裝置");

    otp_login(&pool, &app, SAFARI_IPHONE).await;
    assert_eq!(notices(&pool).await, 1, "沒見過的裝置要通知");

    otp_login(&pool, &app, SAFARI_IPHONE).await;
    assert_eq!(notices(&pool).await, 1, "同一台新裝置只通知一次");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn the_notice_reaches_the_inbox_and_links_to_sessions(pool: PgPool) {
    let app = test_app(&pool);
    otp_login(&pool, &app, CHROME_MAC).await;
    otp_login(&pool, &app, SAFARI_IPHONE).await;
    wishly_api::notify::tick(&pool).await.unwrap();
    let (title, link): (Option<String>, Option<String>) = sqlx::query_as(
        "SELECT inbox_title, inbox_link FROM notifications WHERE kind='auth.new_device'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(title.unwrap().contains("新裝置"));
    assert_eq!(link.as_deref(), Some("/settings"));
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn password_login_from_an_unseen_device_triggers_the_notice(pool: PgPool) {
    let app = test_app(&pool);
    otp_login(&pool, &app, CHROME_MAC).await;
    let (st, _, b) = call_app(
        &app,
        "POST",
        "/auth/password/reset/request",
        &[],
        Some(json!({ "email": EMAIL })),
    )
    .await;
    assert_eq!(st, 202, "{b}");
    sqlx::query("UPDATE otp_challenges SET code_hash=$1 WHERE email=$2 AND consumed_at IS NULL")
        .bind(code_hash("123456", "dev-pepper"))
        .bind(EMAIL)
        .execute(&pool)
        .await
        .unwrap();
    let password = "correct-horse-9";
    let (st, _, b) = call_app(
        &app,
        "POST",
        "/auth/password/reset/confirm",
        &[],
        Some(json!({ "email": EMAIL, "code": "123456", "challenge_id": b["challenge_id"], "new_password": password })),
    )
    .await;
    assert_eq!(st, 204, "{b}");

    let login = json!({ "email": EMAIL, "password": password });
    let same = [("user-agent", CHROME_MAC.to_string())];
    let (st, _, b) = call_app(&app, "POST", "/auth/login", &same, Some(login.clone())).await;
    assert_eq!(st, 200, "{b}");
    assert_eq!(notices(&pool).await, 0, "見過的裝置不通知");

    let other = [("user-agent", SAFARI_IPHONE.to_string())];
    let (st, _, b) = call_app(&app, "POST", "/auth/login", &other, Some(login)).await;
    assert_eq!(st, 200, "{b}");
    assert_eq!(notices(&pool).await, 1, "密碼登入的新裝置要通知");
}

/// 同一支手機在瀏覽器與 LINE 內建瀏覽器間切換是最大的誤報來源，所以 OAuth 登入不通知（D69）。
#[sqlx::test(migrations = "../../db/migrations")]
async fn oauth_login_never_triggers_the_notice(pool: PgPool) {
    let app = test_app(&pool);
    let mock = Router::new()
        .route(
            "/token",
            post(|Form(f): Form<HashMap<String, String>>| async move {
                Json(json!({ "access_token": f["code"] }))
            }),
        )
        .route(
            "/profile",
            get(|| async {
                Json(json!({ "userId": "U-device", "displayName": "LINE 使用者" }))
            }),
        );
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, mock).await.unwrap() });
    std::env::set_var("LINE_CHANNEL_ID", "chan");
    std::env::set_var("LINE_CHANNEL_SECRET", "sec");
    std::env::set_var("LINE_TOKEN_URL", format!("{base}/token"));
    std::env::set_var("LINE_PROFILE_URL", format!("{base}/profile"));

    for ua in [CHROME_MAC, SAFARI_IPHONE] {
        let (st, h, _) = call_app(&app, "GET", "/auth/oauth/line/start", &[], None).await;
        assert_eq!(st, 302);
        let url = reqwest::Url::parse(h[header::LOCATION].to_str().unwrap()).unwrap();
        let state = url
            .query_pairs()
            .find(|(k, _)| k == "state")
            .unwrap()
            .1
            .into_owned();
        let cookie = h[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string();
        let (st, _, _) = call_app(
            &app,
            "GET",
            &format!("/auth/oauth/line/callback?code=c&state={state}"),
            &[("cookie", cookie), ("user-agent", ua.to_string())],
            None,
        )
        .await;
        assert_eq!(st, 302);
    }
    let sessions: i64 = sqlx::query_scalar("SELECT count(*) FROM sessions")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(sessions, 2, "兩次都要真的登入成功");
    assert_eq!(notices(&pool).await, 0);
}
