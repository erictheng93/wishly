//! Q10：刪除帳號前須重新驗證——有密碼者附密碼；沒密碼（Google / LINE）者 session 須是 10 分鐘內建立。
mod common;
use axum::http::StatusCode;
use common::*;
use serde_json::{json, Value};
use sqlx::PgPool;

const PW: &str = "correct-horse-battery";

async fn with_password(pool: &PgPool, name: &str) -> TestUser {
    let u = user(pool, name).await;
    let h = {
        use argon2::{
            password_hash::{rand_core::OsRng, SaltString},
            Argon2, PasswordHasher,
        };
        Argon2::default()
            .hash_password(PW.as_bytes(), &SaltString::generate(&mut OsRng))
            .unwrap()
            .to_string()
    };
    sqlx::query("INSERT INTO auth_identities (user_id, provider, provider_uid, password_hash) SELECT id, 'email', email, $2 FROM users WHERE id=$1")
        .bind(u.id).bind(h).execute(pool).await.unwrap();
    u
}

async fn deleted(pool: &PgPool, u: &TestUser) -> bool {
    sqlx::query_scalar("SELECT deleted_at IS NOT NULL FROM users WHERE id=$1")
        .bind(u.id)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn del(pool: &PgPool, u: &TestUser, body: Value) -> Resp {
    call(pool, "DELETE", "/me", &ck(&u.token), Some(body)).await
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn password_user_must_supply_correct_password(pool: PgPool) {
    let u = with_password(&pool, "有密碼").await;

    let (s, _, b) = del(&pool, &u, json!({"confirm":"DELETE"})).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{b}");
    assert_eq!(b["code"], "VALIDATION_FAILED");
    assert_eq!(b["errors"][0]["pointer"], "/password");

    let (s, _, b) = del(
        &pool,
        &u,
        json!({"confirm":"DELETE","password":"wrong-password"}),
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED, "{b}");
    assert_eq!(b["code"], "INVALID_CREDENTIALS");
    assert!(!deleted(&pool, &u).await);

    let (s, _, b) = del(&pool, &u, json!({"confirm":"DELETE","password":PW})).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert!(deleted(&pool, &u).await);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn wrong_password_is_rate_limited(pool: PgPool) {
    let u = with_password(&pool, "猜密碼").await;
    for _ in 0..5 {
        let (s, _, _) = del(
            &pool,
            &u,
            json!({"confirm":"DELETE","password":"nope-nope"}),
        )
        .await;
        assert_eq!(s, StatusCode::UNAUTHORIZED);
    }
    // 第 6 次即使密碼正確也被擋，不給暴力猜測留驗證機會
    let (s, h, b) = del(&pool, &u, json!({"confirm":"DELETE","password":PW})).await;
    assert_eq!(s, StatusCode::TOO_MANY_REQUESTS, "{b}");
    assert_eq!(b["code"], "RATE_LIMITED");
    assert!(h.contains_key("retry-after"));
    assert!(!deleted(&pool, &u).await);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn passwordless_user_needs_recent_session(pool: PgPool) {
    let old = user(&pool, "舊登入").await;
    sqlx::query("UPDATE sessions SET created_at = now() - interval '11 minutes' WHERE user_id=$1")
        .bind(old.id)
        .execute(&pool)
        .await
        .unwrap();
    let (s, _, b) = del(&pool, &old, json!({"confirm":"DELETE"})).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{b}");
    assert_eq!(b["code"], "REAUTH_REQUIRED");
    assert!(!deleted(&pool, &old).await);

    let fresh = user(&pool, "剛登入").await;
    let (s, _, b) = del(&pool, &fresh, json!({"confirm":"DELETE"})).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert!(deleted(&pool, &fresh).await);
}
