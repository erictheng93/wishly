//! GET /me/security-activity：本人敏感事件、不外露營運身分與理由、不含非白名單動作。
mod common;
use common::*;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

async fn seed(
    pool: &PgPool,
    atype: &str,
    actor: Uuid,
    action: &str,
    entity_id: Uuid,
    diff: serde_json::Value,
) {
    sqlx::query("INSERT INTO audit_logs (actor_type, actor_id, action, entity, entity_id, diff, ip) VALUES ($1::actor_type,$2,$3,'users',$4,$5,'203.0.113.9')")
        .bind(atype).bind(actor).bind(action).bind(entity_id).bind(diff).execute(pool).await.unwrap();
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn security_activity(pool: PgPool) {
    let a = user(&pool, "a").await;
    let b = user(&pool, "b").await;
    let s = staff(&pool, "s").await;
    let app = test_app(&pool);

    seed(
        &pool,
        "user",
        a.id,
        "auth.login",
        a.id,
        json!({"method":"otp","secret":"x"}),
    )
    .await;
    seed(&pool, "user", a.id, "claim.create", a.id, json!({})).await;
    seed(
        &pool,
        "user",
        b.id,
        "auth.login",
        b.id,
        json!({"method":"password"}),
    )
    .await;
    seed(
        &pool,
        "staff",
        s.id,
        "user.mfa_reset",
        a.id,
        json!({"reason":"secret-reason"}),
    )
    .await;
    seed(
        &pool,
        "staff",
        s.id,
        "user.update",
        a.id,
        json!({"reason":"r","before":{"blocked":false},"after":{"blocked":true}}),
    )
    .await;
    seed(&pool, "staff", s.id, "user.update", a.id, json!({"reason":"r","before":{"is_staff":false,"blocked":false},"after":{"is_staff":true,"blocked":false}})).await;

    let (st, h, body) = call_app(&app, "GET", "/me/security-activity", &ck(&a.token), None).await;
    assert_eq!(st, 200, "{body}");
    assert_eq!(h["cache-control"], "no-store");
    let raw = body.to_string();
    assert!(
        !raw.contains("secret") && !raw.contains(&s.id.to_string()),
        "{raw}"
    );
    let mut acts: Vec<&str> = body["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["action"].as_str().unwrap())
        .collect();
    acts.sort();
    assert_eq!(
        acts,
        ["account.suspended", "auth.login", "user.mfa_reset"],
        "{raw}"
    );
    let login = body["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["action"] == "auth.login")
        .unwrap();
    assert_eq!(login["method"], "otp");
    assert_eq!(login["ip"], "203.0.113.9");

    // 分頁
    let (_, _, p1) = call_app(
        &app,
        "GET",
        "/me/security-activity?limit=2",
        &ck(&a.token),
        None,
    )
    .await;
    assert_eq!(p1["data"].as_array().unwrap().len(), 2);
    let cur = p1["next_cursor"].as_str().unwrap();
    let (_, _, p2) = call_app(
        &app,
        "GET",
        &format!("/me/security-activity?limit=2&before={cur}"),
        &ck(&a.token),
        None,
    )
    .await;
    assert_eq!(p2["data"].as_array().unwrap().len(), 1);
    assert!(p2["next_cursor"].is_null());

    let (st, _, _) = call_app(&app, "GET", "/me/security-activity", &[], None).await;
    assert_eq!(st, 401);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn real_login_is_visible(pool: PgPool) {
    let a = user(&pool, "a").await;
    let app = test_app(&pool);
    sqlx::query("UPDATE users SET email='sa@example.com' WHERE id=$1")
        .bind(a.id)
        .execute(&pool)
        .await
        .unwrap();
    let (st, _, b) = call_app(
        &app,
        "POST",
        "/auth/otp/request",
        &[],
        Some(json!({"email":"sa@example.com"})),
    )
    .await;
    assert_eq!(st, 202, "{b}");
    sqlx::query("UPDATE otp_challenges SET code_hash=$1 WHERE email='sa@example.com' AND consumed_at IS NULL")
        .bind(wishly_api::auth::code_hash("123456", "dev-pepper")).execute(&pool).await.unwrap();
    let (st, _, v) = call_app(
        &app,
        "POST",
        "/auth/otp/verify",
        &[],
        Some(json!({"email":"sa@example.com","code":"123456","challenge_id":b["challenge_id"]})),
    )
    .await;
    assert_eq!(st, 200, "{v}");
    let (_, _, body) = call_app(&app, "GET", "/me/security-activity", &ck(&a.token), None).await;
    assert!(
        body["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["action"] == "auth.login" && r["method"] == "otp"),
        "{body}"
    );
}
