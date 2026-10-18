//! 軟上限 + Turnstile：超過 3 次要人機驗證，缺 / 錯 token 回 403 CAPTCHA_REQUIRED。
//! 獨立測試檔（獨立程序）：需在 config 初始化前設環境變數。
mod common;
use axum::{routing::post, Json, Router};
use common::*;
use serde_json::json;
use sqlx::PgPool;

#[sqlx::test(migrations = "../../db/migrations")]
async fn soft_limit_requires_turnstile(pool: PgPool) {
    let mock =
        Router::new().route(
            "/verify",
            post(|body: String| async move {
                Json(json!({ "success": body.contains("response=good") }))
            }),
        );
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(l, mock).await.unwrap() });
    std::env::set_var("TURNSTILE_SECRET", "s3cret");
    std::env::set_var("TURNSTILE_VERIFY_URL", format!("http://{addr}/verify"));
    let app = test_app(&pool);
    for (uri, purpose) in [
        ("/auth/otp/request", "login"),
        ("/auth/password/reset/request", "reset"),
    ] {
        let email = format!("{purpose}-soft@example.com");
        // 前 3 次不需 token（各別 IP 避開 60 秒間隔）
        for i in 0..3 {
            let mut req = request(
                "POST",
                &format!("/api/v1{uri}"),
                &[],
                Some(json!({ "email": email })),
            );
            req.extensions_mut()
                .insert(axum::extract::ConnectInfo::<std::net::SocketAddr>(
                    format!("10.9.{i}.1:1").parse().unwrap(),
                ));
            assert_eq!(call_request(&app, req).await.0, 202, "{uri}");
        }
        let try_with = |tok: Option<&str>, ip: &str| {
            let mut body = json!({ "email": email });
            if let Some(t) = tok {
                body["turnstile_token"] = json!(t);
            }
            let mut req = request("POST", &format!("/api/v1{uri}"), &[], Some(body));
            req.extensions_mut()
                .insert(axum::extract::ConnectInfo::<std::net::SocketAddr>(
                    format!("{ip}:1").parse().unwrap(),
                ));
            call_request(&app, req)
        };
        for tok in [None, Some("bad")] {
            let (st, _, b) = try_with(tok, "10.9.9.1").await;
            assert_eq!(
                (st.as_u16(), b["code"].as_str()),
                (403, Some("CAPTCHA_REQUIRED")),
                "{uri} {tok:?}"
            );
        }
        let (st, _, b) = try_with(Some("good"), "10.9.9.1").await;
        assert_eq!(st, 202, "{uri} {b}");
        assert!(b["challenge_id"].is_string());
    }
}
