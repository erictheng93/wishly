//! LINE OAuth 的本機 provider 整合。
mod common;
use axum::{
    http::header,
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};
use sqlx::PgPool;
use wishsync_api::{app, AppState};

fn setup(pool: PgPool) -> (Router, PgPool) {
    (app(AppState { pool: pool.clone() }), pool)
}

async fn call(
    app: &Router,
    method: &str,
    uri: &str,
    cookie: Option<&str>,
    body: Option<Value>,
) -> common::Resp {
    let h = cookie
        .map(|c| vec![("cookie", c.to_string())])
        .unwrap_or_default();
    common::call(app, common::request(method, uri, &h, body)).await
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn line_login_with_mock_provider(pool: PgPool) {
    let (app, pool) = setup(pool);
    // mock LINE：/token 與 /profile
    let mock = Router::new()
        .route("/token", post(|| async { Json(json!({ "access_token": "at-1" })) }))
        .route("/profile", get(|h: axum::http::HeaderMap| async move {
            assert_eq!(h.get("authorization").unwrap(), "Bearer at-1");
            Json(json!({ "userId": "U-mock-123", "displayName": "LINE小明", "pictureUrl": "https://img.example/p.png" }))
        }));
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, mock).await.unwrap() });
    // 僅此測試使用 LINE_* 環境變數
    std::env::set_var("LINE_CHANNEL_ID", "chan");
    std::env::set_var("LINE_CHANNEL_SECRET", "sec");
    std::env::set_var("LINE_TOKEN_URL", format!("{base}/token"));
    std::env::set_var("LINE_PROFILE_URL", format!("{base}/profile"));

    let (st, h, _) = call(
        &app,
        "GET",
        "/api/v1/auth/oauth/line/start?redirect=/lists",
        None,
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
    assert!(
        loc.starts_with("https://access.line.me/oauth2/v2.1/authorize?")
            && loc.contains("code_challenge_method=S256"),
        "{loc}"
    );
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
    assert_eq!(
        call(
            &app,
            "GET",
            "/api/v1/auth/oauth/line/start?redirect=//evil.com",
            None,
            None
        )
        .await
        .0,
        422
    );

    // state 不符 → 302 login?error=oauth_failed
    let (st, h, _) = call(
        &app,
        "GET",
        "/api/v1/auth/oauth/line/callback?code=c&state=wrong",
        Some(&oc),
        None,
    )
    .await;
    assert_eq!(st, 302);
    assert!(h
        .get(header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap()
        .ends_with("/login?error=oauth_failed"));

    let (st, h, _) = call(
        &app,
        "GET",
        &format!("/api/v1/auth/oauth/line/callback?code=c&state={state}"),
        Some(&oc),
        None,
    )
    .await;
    assert_eq!(st, 302);
    assert!(h
        .get(header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap()
        .ends_with("/lists"));
    let sess = h
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|v| v.to_str().unwrap().to_string())
        .find(|c| c.starts_with("ws_session="))
        .unwrap();
    let cookie = sess.split(';').next().unwrap().to_string();
    let (st, _, me) = call(&app, "GET", "/api/v1/me", Some(&cookie), None).await;
    assert_eq!(st, 200);
    assert_eq!(me["display_name"], "LINE小明");
    assert_eq!(me["identities"][0]["provider"], "line");
    // 第二次登入是同一 user
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM auth_identities WHERE provider='line' AND provider_uid='U-mock-123'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(n, 1);
}
