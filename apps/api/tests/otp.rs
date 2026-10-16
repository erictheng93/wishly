//! OTP 登入、一次性驗證與錯誤次數限制。
mod common;
use axum::http::{header, StatusCode};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use common::*;

/// 固定測試碼，不依賴 SMTP 或暴力枚舉。
async fn recover_code(pool: &PgPool, email: &str) -> String {
    let code = "123456";
    sqlx::query("UPDATE otp_challenges SET code_hash=$1 WHERE email=$2 AND consumed_at IS NULL")
        .bind(wishly_api::auth::code_hash(
            code,
            &wishly_api::config::get().otp_pepper,
        ))
        .bind(email)
        .execute(pool)
        .await
        .unwrap();
    code.to_string()
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn otp_full_flow(pool: PgPool) {
    let app = test_app(&pool);
    let email = format!("otp-{}@Example.com", Uuid::new_v4());
    let lower = email.to_lowercase();
    let (st, _, b) = call_app(
        &app,
        "POST",
        "/auth/otp/request",
        &[],
        Some(json!({ "email": email })),
    )
    .await;
    assert_eq!(st, 202);
    assert_eq!(b["expires_in"], 600);
    // 60 秒內重送 → 429
    let (st, h, b) = call_app(
        &app,
        "POST",
        "/auth/otp/request",
        &[],
        Some(json!({ "email": email })),
    )
    .await;
    assert_eq!(st, 429);
    assert_eq!(b["code"], "RATE_LIMITED");
    assert!(h.contains_key("retry-after"));
    let code = recover_code(&pool, &lower).await;
    let wrong = if code == "000000" { "000001" } else { "000000" };
    let (st, _, b) = call_app(
        &app,
        "POST",
        "/auth/otp/verify",
        &[],
        Some(json!({ "email": email, "code": wrong })),
    )
    .await;
    assert_eq!(
        (st, b["code"].as_str()),
        (StatusCode::BAD_REQUEST, Some("OTP_INVALID"))
    );
    let (st, h, b) = call_app(
        &app,
        "POST",
        "/auth/otp/verify",
        &[],
        Some(json!({ "email": email, "code": code })),
    )
    .await;
    assert_eq!(st, 200, "{b}");
    assert_eq!(b["is_new_user"], true);
    assert_eq!(b["user"]["email"], lower);
    let sc = h
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        sc.starts_with("ws_session=") && sc.contains("HttpOnly") && sc.contains("SameSite=Lax"),
        "{sc}"
    );
    let cookie = sc.split(';').next().unwrap().to_string();
    // 驗證碼只能用一次
    let (st, ..) = call_app(
        &app,
        "POST",
        "/auth/otp/verify",
        &[],
        Some(json!({ "email": email, "code": code })),
    )
    .await;
    assert_eq!(st, 400);
    // /me
    let (st, _, me) = call_app(&app, "GET", "/me", &[("cookie", cookie.clone())], None).await;
    assert_eq!(st, 200);
    assert_eq!(me["identities"][0]["provider"], "email");
    assert_eq!(me["notification_prefs"]["email_claims"], true);
    let (st, _, me) = call_app(
        &app,
        "PATCH",
        "/me",
        &[("cookie", cookie.clone())],
        Some(json!({ "display_name": "小米", "notification_prefs": { "email_claims": false } })),
    )
    .await;
    assert_eq!(st, 200);
    assert_eq!(me["display_name"], "小米");
    assert_eq!(me["notification_prefs"]["email_claims"], false);
    let (st, ..) = call_app(
        &app,
        "PATCH",
        "/me",
        &[("cookie", cookie.clone())],
        Some(json!({ "notification_prefs": { "evil": true } })),
    )
    .await;
    assert_eq!(st, 422);
    // 登出後 session 失效
    let (st, h, _) = call_app(
        &app,
        "POST",
        "/auth/logout",
        &[("cookie", cookie.clone())],
        None,
    )
    .await;
    assert_eq!(st, 204);
    assert!(h
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap()
        .contains("Max-Age=0"));
    let (st, ..) = call_app(&app, "GET", "/me", &[("cookie", cookie.clone())], None).await;
    assert_eq!(st, 401);
    // 再次登入同 email → 同一 user，非新使用者（先把上一張挑戰時間往前挪以略過 60 秒限制）
    sqlx::query(
        "UPDATE otp_challenges SET created_at = created_at - interval '2 minutes' WHERE email=$1",
    )
    .bind(&lower)
    .execute(&pool)
    .await
    .unwrap();
    call_app(
        &app,
        "POST",
        "/auth/otp/request",
        &[],
        Some(json!({ "email": email })),
    )
    .await;
    let code = recover_code(&pool, &lower).await;
    let (_, _, b) = call_app(
        &app,
        "POST",
        "/auth/otp/verify",
        &[],
        Some(json!({ "email": email, "code": code })),
    )
    .await;
    assert_eq!(b["is_new_user"], false);
    assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn otp_locks_after_5_wrong_attempts(pool: PgPool) {
    let app = test_app(&pool);
    let email = format!("lock-{}@example.com", Uuid::new_v4());
    call_app(
        &app,
        "POST",
        "/auth/otp/request",
        &[],
        Some(json!({ "email": email })),
    )
    .await;
    let code = recover_code(&pool, &email).await;
    let wrong = if code == "000000" { "000001" } else { "000000" };
    for _ in 0..5 {
        call_app(
            &app,
            "POST",
            "/auth/otp/verify",
            &[],
            Some(json!({ "email": email, "code": wrong })),
        )
        .await;
    }
    let (st, _, b) = call_app(
        &app,
        "POST",
        "/auth/otp/verify",
        &[],
        Some(json!({ "email": email, "code": code })),
    )
    .await;
    assert_eq!(
        (st, b["code"].as_str()),
        (StatusCode::BAD_REQUEST, Some("OTP_INVALID"))
    );
    let (st, ..) = call_app(
        &app,
        "POST",
        "/auth/otp/request",
        &[],
        Some(json!({ "email": "bad" })),
    )
    .await;
    assert_eq!(st, 422);
    assert_all_reconciled(&pool).await;
}
