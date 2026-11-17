//! 未登入客服表單須通過 Turnstile。獨立測試檔（獨立程序）：需在 config 初始化前設環境變數。
mod common;
use axum::{routing::post, Json, Router};
use common::*;
use serde_json::json;
use sqlx::PgPool;

#[sqlx::test(migrations = "../../db/migrations")]
async fn public_support_request_requires_turnstile(pool: PgPool) {
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
    for (tok, want) in [(None, 403), (Some("bad"), 403), (Some("good"), 201)] {
        let mut body = json!({ "email": "a@example.com", "category": "other", "body": "求助" });
        if let Some(t) = tok {
            body["turnstile_token"] = json!(t);
        }
        let (st, _, v) = call_app(&app, "POST", "/public/support-requests", &[], Some(body)).await;
        assert_eq!(st.as_u16(), want, "{tok:?} {v}");
        if want == 403 {
            assert_eq!(v["code"], "CAPTCHA_REQUIRED");
        }
    }
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM support_requests")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 1);
}
