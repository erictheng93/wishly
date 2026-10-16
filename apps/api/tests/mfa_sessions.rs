//! 帳號安全：營運人員 TOTP（軟性：啟用後才強制）、登入裝置管理、登入後改密碼、安全事件稽核。
mod common;
use common::{call, ck};
use hmac::{Hmac, Mac};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

fn hotp(secret: &[u8], step: i64) -> String {
    let mut m = <Hmac<sha1::Sha1> as Mac>::new_from_slice(secret).unwrap();
    m.update(&(step as u64).to_be_bytes());
    let h = m.finalize().into_bytes();
    let o = (h[19] & 0x0f) as usize;
    let n = (u32::from_be_bytes([h[o], h[o + 1], h[o + 2], h[o + 3]]) & 0x7fff_ffff) % 1_000_000;
    format!("{n:06}")
}
fn now_step() -> i64 {
    chrono::Utc::now().timestamp() / 30
}
async fn secret(pool: &PgPool, uid: Uuid) -> Vec<u8> {
    let s: Vec<u8> = sqlx::query_scalar("SELECT totp_secret FROM users WHERE id=$1")
        .bind(uid)
        .fetch_one(pool)
        .await
        .unwrap();
    wishly_api::sealed::open(&s).unwrap()
}
async fn audit_count(pool: &PgPool, uid: Uuid, action: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE actor_id=$1 AND action=$2")
        .bind(uid)
        .bind(action)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn staff_totp_soft_then_enforced(pool: PgPool) {
    let s = common::staff(&pool, "ops").await;
    let u = common::user(&pool, "u").await;
    // 未啟用：照常進後台，/me 顯示未啟用
    assert_eq!(
        call(&pool, "GET", "/admin/stats", &ck(&s.token), None)
            .await
            .0,
        200
    );
    let me = call(&pool, "GET", "/me", &ck(&s.token), None).await.2;
    assert_eq!(
        (me["mfa_enabled"].as_bool(), me["mfa_verified"].as_bool()),
        (Some(false), Some(false))
    );
    // 一般使用者不能設定
    let r = call(&pool, "POST", "/auth/mfa/totp/setup", &ck(&u.token), None).await;
    assert_eq!(
        (r.0.as_u16(), r.2["code"].as_str()),
        (403, Some("STAFF_ONLY"))
    );

    let r = call(&pool, "POST", "/auth/mfa/totp/setup", &ck(&s.token), None).await;
    assert_eq!(r.0, 200);
    assert!(r.2["otpauth_uri"]
        .as_str()
        .unwrap()
        .starts_with("otpauth://totp/Wishly:"));
    let k = secret(&pool, s.id).await;
    let step = now_step();
    let bad = call(
        &pool,
        "POST",
        "/auth/mfa/totp/enable",
        &ck(&s.token),
        Some(json!({"code":"000000"})),
    )
    .await;
    if hotp(&k, step) != "000000" {
        assert_eq!(bad.2["code"], "MFA_CODE_INVALID");
    }
    let r = call(
        &pool,
        "POST",
        "/auth/mfa/totp/enable",
        &ck(&s.token),
        Some(json!({"code": hotp(&k, step)})),
    )
    .await;
    assert_eq!(r.0, 204, "{:?}", r.2);
    assert_eq!(audit_count(&pool, s.id, "auth.mfa_enabled").await, 1);
    // 已啟用後不可重新 setup
    assert_eq!(
        call(&pool, "POST", "/auth/mfa/totp/setup", &ck(&s.token), None)
            .await
            .2["code"],
        "MFA_ALREADY_ENABLED"
    );
    // 啟用的 session 可直接用後台
    assert_eq!(
        call(&pool, "GET", "/admin/stats", &ck(&s.token), None)
            .await
            .0,
        200
    );

    // 新 session：未驗證 → MFA_REQUIRED；同一碼不能重放；下一步的碼可通過
    let t2 = common::session(&pool, s.id).await;
    let r = call(&pool, "GET", "/admin/stats", &ck(&t2), None).await;
    assert_eq!(
        (r.0.as_u16(), r.2["code"].as_str()),
        (403, Some("MFA_REQUIRED"))
    );
    let r = call(
        &pool,
        "POST",
        "/auth/mfa/verify",
        &ck(&t2),
        Some(json!({"code": hotp(&k, step)})),
    )
    .await;
    assert_eq!(r.2["code"], "MFA_CODE_INVALID", "同一時間步不得重放");
    let r = call(
        &pool,
        "POST",
        "/auth/mfa/verify",
        &ck(&t2),
        Some(json!({"code": hotp(&k, now_step() + 1)})),
    )
    .await;
    assert_eq!(r.0, 204, "{:?}", r.2);
    assert_eq!(
        call(&pool, "GET", "/admin/stats", &ck(&t2), None).await.0,
        200
    );
    assert_eq!(audit_count(&pool, s.id, "auth.mfa_verified").await, 1);

    // 另一位營運人員重設：清除密鑰並撤銷 session
    let s2 = common::staff(&pool, "ops2").await;
    let r = call(
        &pool,
        "DELETE",
        &format!("/admin/users/{}/mfa", s.id),
        &ck(&s2.token),
        Some(json!({"reason":"lost phone"})),
    )
    .await;
    assert_eq!(r.0, 204, "{:?}", r.2);
    assert_eq!(call(&pool, "GET", "/me", &ck(&t2), None).await.0, 401);
    let t3 = common::session(&pool, s.id).await;
    assert_eq!(
        call(&pool, "GET", "/admin/stats", &ck(&t3), None).await.0,
        200,
        "重設後回到未啟用"
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn mfa_verify_rate_limited(pool: PgPool) {
    let s = common::staff(&pool, "ops").await;
    call(&pool, "POST", "/auth/mfa/totp/setup", &ck(&s.token), None).await;
    let k = secret(&pool, s.id).await;
    let wrong = if hotp(&k, now_step()) == "123456" {
        "654321"
    } else {
        "123456"
    };
    for _ in 0..5 {
        call(
            &pool,
            "POST",
            "/auth/mfa/totp/enable",
            &ck(&s.token),
            Some(json!({"code": wrong})),
        )
        .await;
    }
    let r = call(
        &pool,
        "POST",
        "/auth/mfa/totp/enable",
        &ck(&s.token),
        Some(json!({"code": hotp(&k, now_step())})),
    )
    .await;
    assert_eq!(r.0, 429);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn device_list_and_revoke(pool: PgPool) {
    let a = common::user(&pool, "a").await;
    let other = common::session(&pool, a.id).await;
    let third = common::session(&pool, a.id).await;
    let b = common::user(&pool, "b").await;

    let r = call(&pool, "GET", "/me/sessions", &ck(&a.token), None).await;
    let list = r.2["data"].as_array().unwrap().clone();
    assert_eq!(list.len(), 3);
    assert_eq!(list.iter().filter(|s| s["current"] == true).count(), 1);
    let other_id: Uuid = sqlx::query_scalar("SELECT id FROM sessions WHERE token_hash=$1")
        .bind(wishly_api::session::hash_token(&other))
        .fetch_one(&pool)
        .await
        .unwrap();

    // 不能撤銷別人的
    assert_eq!(
        call(
            &pool,
            "DELETE",
            &format!("/me/sessions/{other_id}"),
            &ck(&b.token),
            None
        )
        .await
        .0,
        404
    );
    assert_eq!(
        call(
            &pool,
            "DELETE",
            &format!("/me/sessions/{other_id}"),
            &ck(&a.token),
            None
        )
        .await
        .0,
        204
    );
    assert_eq!(call(&pool, "GET", "/me", &ck(&other), None).await.0, 401);
    assert_eq!(audit_count(&pool, a.id, "auth.session_revoked").await, 1);

    let r = call(&pool, "DELETE", "/me/sessions", &ck(&a.token), None).await;
    assert_eq!(r.2["revoked"], 1);
    assert_eq!(call(&pool, "GET", "/me", &ck(&third), None).await.0, 401);
    assert_eq!(
        call(&pool, "GET", "/me", &ck(&a.token), None).await.0,
        200,
        "目前裝置保持登入"
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn change_password_requires_current_and_revokes_others(pool: PgPool) {
    let a = common::user(&pool, "a").await;
    let other = common::session(&pool, a.id).await;
    let email: String = sqlx::query_scalar("SELECT email FROM users WHERE id=$1")
        .bind(a.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    let body = |cur: &str, new: &str| Some(json!({"current_password": cur, "new_password": new}));

    // 沒有 Email 密碼 → 走忘記密碼流程
    let r = call(
        &pool,
        "POST",
        "/me/password",
        &ck(&a.token),
        body("x", "new-password-1"),
    )
    .await;
    assert_eq!(r.2["code"], "PASSWORD_NOT_SET");

    let h = {
        use argon2::{
            password_hash::{rand_core::OsRng, SaltString},
            Argon2, PasswordHasher,
        };
        Argon2::default()
            .hash_password(b"old-password-1", &SaltString::generate(&mut OsRng))
            .unwrap()
            .to_string()
    };
    sqlx::query("INSERT INTO auth_identities (user_id,provider,provider_uid,email,password_hash) VALUES ($1,'email',$2,$2,$3)")
        .bind(a.id).bind(&email).bind(&h).execute(&pool).await.unwrap();
    assert_eq!(
        call(&pool, "GET", "/me", &ck(&a.token), None).await.2["has_password"],
        true
    );
    assert_eq!(
        call(
            &pool,
            "POST",
            "/me/password",
            &ck(&a.token),
            body("old-password-1", "short")
        )
        .await
        .0,
        422
    );
    assert_eq!(
        call(
            &pool,
            "POST",
            "/me/password",
            &ck(&a.token),
            body("wrong-password", "new-password-1")
        )
        .await
        .0,
        422
    );
    assert_eq!(
        call(&pool, "GET", "/me", &ck(&other), None).await.0,
        200,
        "失敗不影響其他裝置"
    );

    let r = call(
        &pool,
        "POST",
        "/me/password",
        &ck(&a.token),
        body("old-password-1", "new-password-1"),
    )
    .await;
    assert_eq!(r.0, 204, "{:?}", r.2);
    assert_eq!(call(&pool, "GET", "/me", &ck(&other), None).await.0, 401);
    assert_eq!(call(&pool, "GET", "/me", &ck(&a.token), None).await.0, 200);
    let r = call(
        &pool,
        "POST",
        "/auth/login",
        &[],
        Some(json!({"email": email, "password": "new-password-1"})),
    )
    .await;
    assert_eq!(r.0, 200);
    let mail: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM notifications WHERE user_id=$1 AND kind='auth.password_changed'",
    )
    .bind(a.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(mail, 1);
    assert_eq!(audit_count(&pool, a.id, "auth.password_changed").await, 1);
    let method: String = sqlx::query_scalar(
        "SELECT diff->>'method' FROM audit_logs WHERE actor_id=$1 AND action='auth.login'",
    )
    .bind(a.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(method, "password");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn logout_is_audited(pool: PgPool) {
    let a = common::user(&pool, "a").await;
    assert_eq!(
        call(&pool, "POST", "/auth/logout", &ck(&a.token), None)
            .await
            .0,
        204
    );
    assert_eq!(audit_count(&pool, a.id, "auth.logout").await, 1);
    assert_eq!(call(&pool, "GET", "/me", &ck(&a.token), None).await.0, 401);
}
