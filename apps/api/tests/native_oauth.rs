//! 原生殼 OAuth：start?native=true → callback 發一次性碼 → /auth/native/exchange（PKCE）。
mod common;
use axum::{
    http::header,
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD as B64, Engine};
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::PgPool;

use common::*;

const TAURI: &str = "tauri://localhost";

fn challenge(verifier: &str) -> String {
    B64.encode(Sha256::digest(verifier.as_bytes()))
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn native_oauth_code_exchange(pool: PgPool) {
    let app = test_app(&pool);
    let mock = Router::new()
        .route(
            "/token",
            post(|| async { Json(json!({ "access_token": "at-1" })) }),
        )
        .route(
            "/profile",
            get(|| async {
                Json(json!({ "userId": "U-native-1", "displayName": "原生小明" }))
            }),
        );
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, mock).await.unwrap() });
    std::env::set_var("LINE_CHANNEL_ID", "chan");
    std::env::set_var("LINE_CHANNEL_SECRET", "sec");
    std::env::set_var("LINE_TOKEN_URL", format!("{base}/token"));
    std::env::set_var("LINE_PROFILE_URL", format!("{base}/profile"));

    let verifier = "v".repeat(43);
    let ch = challenge(&verifier);

    // 缺 / 格式錯的 challenge、與 link 併用都擋下
    for q in [
        "native=true",
        "native=true&app_challenge=short",
        &format!("native=true&link=true&app_challenge={ch}"),
    ] {
        let (st, _, _) = call_app(
            &app,
            "GET",
            &format!("/auth/oauth/line/start?{q}"),
            &[],
            None,
        )
        .await;
        assert_eq!(st, 422, "{q}");
    }

    let start = |ch: String| {
        let app = app.clone();
        async move {
            let (st, h, _) = call_app(
                &app,
                "GET",
                &format!("/auth/oauth/line/start?native=true&app_challenge={ch}"),
                &[],
                None,
            )
            .await;
            assert_eq!(st, 302);
            let loc = h
                .get(header::LOCATION)
                .unwrap()
                .to_str()
                .unwrap()
                .to_string();
            let state = loc
                .split("state=")
                .nth(1)
                .unwrap()
                .split('&')
                .next()
                .unwrap()
                .to_string();
            let oc = h
                .get(header::SET_COOKIE)
                .unwrap()
                .to_str()
                .unwrap()
                .split(';')
                .next()
                .unwrap()
                .to_string();
            (state, oc)
        }
    };
    let callback = |state: String, oc: String| {
        let app = app.clone();
        async move {
            let (st, h, _) = call_app(
                &app,
                "GET",
                &format!("/auth/oauth/line/callback?code=c&state={state}"),
                &[("cookie", oc)],
                None,
            )
            .await;
            assert_eq!(st, 302);
            let loc = h
                .get(header::LOCATION)
                .unwrap()
                .to_str()
                .unwrap()
                .to_string();
            // 瀏覽器端不發 session cookie
            assert!(h
                .get_all(header::SET_COOKIE)
                .iter()
                .all(|c| !c.to_str().unwrap().starts_with("ws_session=")));
            loc
        }
    };
    let exchange = |code: String, verifier: String, origin: Option<&'static str>| {
        let app = app.clone();
        async move {
            let mut hdr = vec![("x-native-client", "1".to_string())];
            if let Some(o) = origin {
                hdr.push(("origin", o.to_string()));
            }
            call_app(
                &app,
                "POST",
                "/auth/native/exchange",
                &hdr,
                Some(json!({ "code": code, "verifier": verifier })),
            )
            .await
        }
    };

    let (state, oc) = start(ch.clone()).await;
    let loc = callback(state, oc).await;
    let code = loc
        .strip_prefix("wishsync://auth?code=")
        .expect(&loc)
        .to_string();

    // 網頁 origin 不能兌換（且不消耗碼）
    assert_eq!(
        exchange(code.clone(), verifier.clone(), Some("https://evil.example"))
            .await
            .0,
        403
    );
    // verifier 錯 → 400，且碼已作廢
    assert_eq!(
        exchange(code.clone(), "x".repeat(43), Some(TAURI)).await.0,
        400
    );
    assert_eq!(exchange(code, verifier.clone(), Some(TAURI)).await.0, 400);

    // 正常流程：兌換拿到權杖，Bearer 可用，且碼不能重放
    let (state, oc) = start(ch).await;
    let code = callback(state, oc)
        .await
        .strip_prefix("wishsync://auth?code=")
        .unwrap()
        .to_string();
    let (st, _, body) = exchange(code.clone(), verifier.clone(), Some(TAURI)).await;
    assert_eq!(st, 200, "{body}");
    let token = body["session_token"].as_str().unwrap().to_string();
    let (st, _, me) = call_app(
        &app,
        "GET",
        "/me",
        &[("authorization", format!("Bearer {token}"))],
        None,
    )
    .await;
    assert_eq!(st, 200);
    assert_eq!(me["display_name"], "原生小明");
    assert_eq!(exchange(code, verifier.clone(), Some(TAURI)).await.0, 400);

    // 過期的碼不能兌換
    let (state, oc) = start(challenge(&verifier)).await;
    let code = callback(state, oc)
        .await
        .strip_prefix("wishsync://auth?code=")
        .unwrap()
        .to_string();
    sqlx::query("UPDATE native_auth_codes SET expires_at = now() - interval '1 second'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(exchange(code, verifier, Some(TAURI)).await.0, 400);

    // 供應商失敗 → 以 deep link 回報錯誤
    let (state, oc) = start(challenge(&"z".repeat(43))).await;
    let (_, h, _) = call_app(
        &app,
        "GET",
        &format!("/auth/oauth/line/callback?error=access_denied&state={state}"),
        &[("cookie", oc)],
        None,
    )
    .await;
    assert_eq!(
        h.get(header::LOCATION).unwrap(),
        "wishsync://auth?error=oauth_failed"
    );
}
