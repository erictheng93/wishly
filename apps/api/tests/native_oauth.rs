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

// LINE_* 是行程全域環境變數；兩個測試各自的 mock 不能同時生效
static ENV: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

const TAURI: &str = "tauri://localhost";

fn challenge(verifier: &str) -> String {
    B64.encode(Sha256::digest(verifier.as_bytes()))
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn native_oauth_code_exchange(pool: PgPool) {
    let _env = ENV.lock().await;
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

#[sqlx::test(migrations = "../../db/migrations")]
async fn native_link_ticket_flow(pool: PgPool) {
    let _env = ENV.lock().await;
    let app = test_app(&pool);
    let mock = Router::new()
        .route(
            "/token",
            post(|| async { Json(json!({ "access_token": "at-1" })) }),
        )
        .route(
            "/profile",
            get(|| async { Json(json!({ "userId": "U-link-9", "displayName": "要綁的LINE" })) }),
        );
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, mock).await.unwrap() });
    std::env::set_var("LINE_CHANNEL_ID", "chan");
    std::env::set_var("LINE_CHANNEL_SECRET", "sec");
    std::env::set_var("LINE_TOKEN_URL", format!("{base}/token"));
    std::env::set_var("LINE_PROFILE_URL", format!("{base}/profile"));

    let me = user(&pool, "綁定者").await;
    let token = session(&pool, me.id).await;
    let bearer = ("authorization", format!("Bearer {token}"));
    let native = [
        ("origin", TAURI.to_string()),
        ("x-native-client", "1".to_string()),
        bearer.clone(),
    ];
    let ch = challenge(&"v".repeat(43));

    // 沒帶 ticket、或 ticket 不存在 → 擋下
    let url = format!("/auth/oauth/line/start?native=true&link=true&app_challenge={ch}");
    assert_eq!(call_app(&app, "GET", &url, &[], None).await.0, 422);
    assert_eq!(
        call_app(&app, "GET", &format!("{url}&ticket=nope"), &[], None)
            .await
            .0,
        401
    );

    // 網頁 origin 不能換 ticket；未登入也不行
    let web = [
        ("origin", "https://evil.example".to_string()),
        ("x-native-client", "1".to_string()),
        bearer.clone(),
    ];
    let body = Some(json!({ "app_challenge": ch }));
    assert_eq!(
        call_app(
            &app,
            "POST",
            "/auth/oauth/line/link-ticket",
            &web,
            body.clone()
        )
        .await
        .0,
        403
    );
    let anon = [
        ("origin", TAURI.to_string()),
        ("x-native-client", "1".to_string()),
    ];
    assert_eq!(
        call_app(
            &app,
            "POST",
            "/auth/oauth/line/link-ticket",
            &anon,
            body.clone()
        )
        .await
        .0,
        401
    );

    let get_ticket = || async {
        let (st, _, b) = call_app(
            &app,
            "POST",
            "/auth/oauth/line/link-ticket",
            &native,
            body.clone(),
        )
        .await;
        assert_eq!(st, 200, "{b}");
        b["ticket"].as_str().unwrap().to_string()
    };
    // ticket 綁 app_challenge：別的 challenge 用不了（且已被消耗）
    let t = get_ticket().await;
    let other = challenge(&"o".repeat(43));
    let wrong =
        format!("/auth/oauth/line/start?native=true&link=true&app_challenge={other}&ticket={t}");
    assert_eq!(call_app(&app, "GET", &wrong, &[], None).await.0, 401);

    // 登入碼不能當 ticket；ticket 不能拿去兌換 session
    let t = get_ticket().await;
    let (st, _, _) = call_app(
        &app,
        "POST",
        "/auth/native/exchange",
        &[
            ("origin", TAURI.to_string()),
            ("x-native-client", "1".to_string()),
        ],
        Some(json!({ "code": t, "verifier": "v".repeat(43) })),
    )
    .await;
    assert_eq!(st, 400);

    // 正常流程（瀏覽器端不帶任何 session）→ deep link ?linked=line，且身分綁到該使用者
    let t = get_ticket().await;
    let (st, h, _) = call_app(&app, "GET", &format!("{url}&ticket={t}"), &[], None).await;
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
    // 同一張 ticket 不能重放
    assert_eq!(
        call_app(&app, "GET", &format!("{url}&ticket={t}"), &[], None)
            .await
            .0,
        401
    );
    let (st, h, _) = call_app(
        &app,
        "GET",
        &format!("/auth/oauth/line/callback?code=c&state={state}"),
        &[("cookie", oc)],
        None,
    )
    .await;
    assert_eq!(st, 302);
    assert_eq!(
        h.get(header::LOCATION).unwrap(),
        "wishsync://auth?linked=line"
    );
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM auth_identities WHERE user_id=$1 AND provider='line' AND provider_uid='U-link-9'")
        .bind(me.id).fetch_one(&pool).await.unwrap();
    assert_eq!(n, 1);

    // ticket 發出後 session 被登出 → 綁定在 callback 失敗並以 deep link 回報
    let me2 = user(&pool, "登出者").await;
    let tok2 = session(&pool, me2.id).await;
    let (_, _, b) = call_app(
        &app,
        "POST",
        "/auth/oauth/line/link-ticket",
        &[
            ("origin", TAURI.to_string()),
            ("x-native-client", "1".to_string()),
            ("authorization", format!("Bearer {tok2}")),
        ],
        body.clone(),
    )
    .await;
    let t2 = b["ticket"].as_str().unwrap().to_string();
    let (_, h, _) = call_app(&app, "GET", &format!("{url}&ticket={t2}"), &[], None).await;
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
    sqlx::query("UPDATE sessions SET revoked_at = now() WHERE user_id=$1")
        .bind(me2.id)
        .execute(&pool)
        .await
        .unwrap();
    let (_, h, _) = call_app(
        &app,
        "GET",
        &format!("/auth/oauth/line/callback?code=c&state={state}"),
        &[("cookie", oc)],
        None,
    )
    .await;
    assert_eq!(
        h.get(header::LOCATION).unwrap(),
        "wishsync://auth?error=oauth_failed"
    );
}
