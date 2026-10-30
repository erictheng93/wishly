//! 綁定後換另一種方式登入，必須回到同一個帳號（完整走 HTTP，LINE 用本機 mock）。
mod common;
use axum::{
    http::{header, HeaderMap},
    routing::{get, post},
    Form, Json, Router,
};
use serde_json::{json, Value};
use sqlx::PgPool;
use std::collections::HashMap;
use uuid::Uuid;
use wishly_api::auth::code_hash;

use common::*;

/// 寫死驗證碼後送出，不依賴寄信。回 (status, headers, body)。
async fn with_code(pool: &PgPool, app: &Router, email: &str, uri: &str, mut body: Value) -> Resp {
    sqlx::query("UPDATE otp_challenges SET code_hash=$1 WHERE email=$2 AND consumed_at IS NULL")
        .bind(code_hash("123456", "dev-pepper"))
        .bind(email)
        .execute(pool)
        .await
        .unwrap();
    body["code"] = json!("123456");
    call_app(app, "POST", uri, &[], Some(body)).await
}

async fn otp_login(pool: &PgPool, app: &Router, email: &str) -> Resp {
    let (st, _, b) = call_app(
        app,
        "POST",
        "/auth/otp/request",
        &[],
        Some(json!({ "email": email })),
    )
    .await;
    assert_eq!(st, 202, "{b}");
    let body = json!({ "email": email, "challenge_id": b["challenge_id"] });
    with_code(pool, app, email, "/auth/otp/verify", body).await
}

fn session_of(h: &HeaderMap) -> String {
    h.get_all(header::SET_COOKIE)
        .iter()
        .map(|v| v.to_str().unwrap())
        .find(|c| c.starts_with("ws_session="))
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string()
}

async fn me(app: &Router, cookie: &str) -> Value {
    let (st, _, b) = call_app(app, "GET", "/me", &[("cookie", cookie.into())], None).await;
    assert_eq!(st, 200, "{b}");
    b
}

/// 走完 LINE OAuth；`session` 有值時是已登入綁定（?link=true）。回 callback 的 headers。
async fn line_flow(app: &Router, code: &str, session: Option<&str>) -> HeaderMap {
    let hdr: Hdr = session
        .map(|s| ("cookie", s.to_string()))
        .into_iter()
        .collect();
    let start = if session.is_some() {
        "/auth/oauth/line/start?link=true"
    } else {
        "/auth/oauth/line/start"
    };
    let (st, h, _) = call_app(app, "GET", start, &hdr, None).await;
    assert_eq!(st, 302);
    let url = reqwest::Url::parse(h[header::LOCATION].to_str().unwrap()).unwrap();
    let state = url
        .query_pairs()
        .find(|(k, _)| k == "state")
        .unwrap()
        .1
        .into_owned();
    let oc = h[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();
    let cookie = session.map_or(oc.clone(), |s| format!("{oc}; {s}"));
    let (st, h, _) = call_app(
        app,
        "GET",
        &format!("/auth/oauth/line/callback?code={code}&state={state}"),
        &[("cookie", cookie)],
        None,
    )
    .await;
    assert_eq!(st, 302);
    h
}

async fn users(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM users WHERE deleted_at IS NULL")
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn linked_methods_log_in_to_the_same_account(pool: PgPool) {
    let app = test_app(&pool);
    // mock LINE：userId 由授權碼決定，模擬不同 LINE 使用者
    let mock = Router::new()
        .route(
            "/token",
            post(|Form(f): Form<HashMap<String, String>>| async move {
                Json(json!({ "access_token": f["code"] }))
            }),
        )
        .route(
            "/profile",
            get(|h: HeaderMap| async move {
                let at = h["authorization"]
                    .to_str()
                    .unwrap()
                    .trim_start_matches("Bearer ");
                Json(json!({ "userId": format!("U-{at}"), "displayName": "LINE 使用者" }))
            }),
        );
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, mock).await.unwrap() });
    std::env::set_var("LINE_CHANNEL_ID", "chan");
    std::env::set_var("LINE_CHANNEL_SECRET", "sec");
    std::env::set_var("LINE_TOKEN_URL", format!("{base}/token"));
    std::env::set_var("LINE_PROFILE_URL", format!("{base}/profile"));

    // 情境一：Email 開戶 → 綁 LINE → 用 LINE 登入回同一帳號
    let (st, h, b) = otp_login(&pool, &app, "first@example.com").await;
    assert_eq!(st, 200, "{b}");
    assert_eq!(b["is_new_user"], true);
    let email_first = session_of(&h);
    let id = me(&app, &email_first).await["id"].clone();
    let h = line_flow(&app, "alice", Some(&email_first)).await;
    assert!(h[header::LOCATION]
        .to_str()
        .unwrap()
        .ends_with("/settings?linked=line"));
    let h = line_flow(&app, "alice", None).await;
    assert_eq!(me(&app, &session_of(&h)).await["id"], id);
    assert_eq!(users(&pool).await, 1);

    // 情境二：LINE 開戶（無 Email）→ 綁 Email → 用 Email 登入回同一帳號
    let h = line_flow(&app, "bob", None).await;
    let line_first = session_of(&h);
    let u = me(&app, &line_first).await;
    assert!(u["email"].is_null());
    assert_eq!(u["has_password"], false);
    let email = "bob@example.com";
    let ck = [("cookie", line_first.clone())];
    let (st, _, _) = call_app(
        &app,
        "POST",
        "/me/identities/email/request",
        &ck,
        Some(json!({ "email": email })),
    )
    .await;
    assert_eq!(st, 202);
    sqlx::query("UPDATE otp_challenges SET code_hash=$1 WHERE email=$2")
        .bind(code_hash("123456", "dev-pepper"))
        .bind(email)
        .execute(&pool)
        .await
        .unwrap();
    let (st, _, b) = call_app(
        &app,
        "POST",
        "/me/identities/email/verify",
        &ck,
        Some(json!({ "email": email, "code": "123456" })),
    )
    .await;
    assert_eq!(st, 200, "{b}");
    assert_eq!(users(&pool).await, 2);

    // 綁定只驗證信箱、不設密碼：登入頁的密碼登入此時會失敗
    let pw = json!({ "email": email, "password": "correct-horse-9" });
    let (st, _, _) = call_app(&app, "POST", "/auth/login", &[], Some(pw.clone())).await;
    assert_eq!(st, 401);

    // Email 驗證碼登入回同一帳號
    let (st, h, b) = otp_login(&pool, &app, email).await;
    assert_eq!(st, 200, "{b}");
    assert_eq!(b["is_new_user"], false);
    assert_eq!(me(&app, &session_of(&h)).await["id"], u["id"]);

    // 走「忘記密碼」設定密碼後，密碼登入也回同一帳號
    let (st, _, b) = call_app(
        &app,
        "POST",
        "/auth/password/reset/request",
        &[],
        Some(json!({ "email": email })),
    )
    .await;
    assert_eq!(st, 202, "{b}");
    let body = json!({ "email": email, "challenge_id": b["challenge_id"], "new_password": "correct-horse-9" });
    let (st, _, b) = with_code(&pool, &app, email, "/auth/password/reset/confirm", body).await;
    assert_eq!(st, 204, "{b}");
    let (st, h, b) = call_app(&app, "POST", "/auth/login", &[], Some(pw)).await;
    assert_eq!(st, 200, "{b}");
    assert_eq!(me(&app, &session_of(&h)).await["id"], u["id"]);

    // LINE 也還是同一帳號，全程沒有多開
    let h = line_flow(&app, "bob", None).await;
    assert_eq!(me(&app, &session_of(&h)).await["id"], u["id"]);
    assert_eq!(users(&pool).await, 2);
    let linked: Vec<Uuid> = sqlx::query_scalar(
        "SELECT user_id FROM auth_identities WHERE provider_uid IN ('U-bob', 'bob@example.com')",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(linked.len(), 2);
    assert_eq!(linked[0], linked[1]);
}
