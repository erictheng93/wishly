//! 回歸：唯讀模式放行登入 / OAuth 未設憑證導向 / seed_demo 帳號可登入
mod common;
use axum::http::StatusCode;
use serde_json::json;
use sqlx::PgPool;

async fn set_ro(pool: &PgPool, on: bool) {
    sqlx::query("UPDATE system_flags SET value = to_jsonb($1::bool) WHERE key='read_only'")
        .bind(on)
        .execute(pool)
        .await
        .unwrap();
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn read_only_lets_login_and_otp_through(pool: PgPool) {
    set_ro(&pool, true).await;
    let login = json!({ "email": "nobody@example.com", "password": "whatever123" });
    let (s, _, v) = common::call(&pool, "POST", "/auth/login", &[], Some(login.clone())).await;
    assert_ne!(s, StatusCode::SERVICE_UNAVAILABLE, "{v}");
    let (s, _, v) = common::call(
        &pool,
        "POST",
        "/auth/otp/request",
        &[],
        Some(json!({ "email": "nobody@example.com" })),
    )
    .await;
    assert_ne!(s, StatusCode::SERVICE_UNAVAILABLE, "{v}");
    let (s, _, v) = common::call(
        &pool,
        "POST",
        "/wishlists",
        &[],
        Some(json!({ "title": "x" })),
    )
    .await;
    assert_eq!(
        (s, v["code"].as_str()),
        (StatusCode::SERVICE_UNAVAILABLE, Some("READ_ONLY_MODE")),
        "{v}"
    );
    assert_ne!(
        common::call(&pool, "GET", "/wishlists", &[], None).await.0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    // 關閉後恢復：寫入不再是 503（未登入 → 401 之類）
    set_ro(&pool, false).await;
    let (s, _, v) = common::call(
        &pool,
        "POST",
        "/wishlists",
        &[],
        Some(json!({ "title": "x" })),
    )
    .await;
    assert_ne!(s, StatusCode::SERVICE_UNAVAILABLE, "{v}");
    common::assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn oauth_start_without_credentials_redirects(pool: PgPool) {
    // ponytail: 同一 binary 內其他測試不設這些 env；若日後有，改用 serial 鎖
    let keys = [
        "GOOGLE_CLIENT_ID",
        "GOOGLE_CLIENT_SECRET",
        "LINE_CHANNEL_ID",
        "LINE_CHANNEL_SECRET",
    ];
    let app_url = std::env::var("APP_URL")
        .unwrap_or("http://localhost:3000".into())
        .trim_end_matches('/')
        .to_string();
    // 未設定與設成空字串（例如部署範本留空）都要視為未設定
    for empty in [false, true] {
        for k in keys {
            if empty {
                std::env::set_var(k, "")
            } else {
                std::env::remove_var(k)
            }
        }
        for p in ["google", "line"] {
            let (s, h, _) =
                common::call(&pool, "GET", &format!("/auth/oauth/{p}/start"), &[], None).await;
            assert_eq!(s, StatusCode::FOUND, "{p} empty={empty}");
            assert_eq!(
                h["location"],
                format!("{app_url}/login?error=oauth_unavailable"),
                "{p} empty={empty}"
            );
        }
    }
    for k in keys {
        std::env::remove_var(k);
    }
    common::assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn otp_cooldown_ignores_consumed_challenges(pool: PgPool) {
    let req = || {
        common::call(
            &pool,
            "POST",
            "/auth/otp/request",
            &[],
            Some(json!({ "email": "cool@example.com" })),
        )
    };
    // 剛驗證成功（已使用）的驗證碼不擋下一次請求
    sqlx::query("INSERT INTO otp_challenges (email, code_hash, expires_at, consumed_at) VALUES ('cool@example.com', '\\x00', now() + interval '10 minutes', now())").execute(&pool).await.unwrap();
    assert_eq!(req().await.0, StatusCode::ACCEPTED);
    // 但剛寄出、尚未使用的驗證碼，60 秒內再要一次仍被擋
    assert_eq!(req().await.0, StatusCode::TOO_MANY_REQUESTS);
    common::assert_all_reconciled(&pool).await;
}

/// 複製 examples/seed_demo.rs 的帳號寫入邏輯
#[sqlx::test(migrations = "../../db/migrations")]
async fn seed_demo_account_can_login(pool: PgPool) {
    use argon2::{
        password_hash::{rand_core::OsRng, SaltString},
        Argon2, PasswordHasher,
    };
    let hash = Argon2::default()
        .hash_password(b"demo123", &SaltString::generate(&mut OsRng))
        .unwrap()
        .to_string();
    let uid: uuid::Uuid = sqlx::query_scalar("INSERT INTO users (display_name, email) VALUES ('Demo', 'demo@demo.com') ON CONFLICT (email) DO UPDATE SET deleted_at = NULL RETURNING id").fetch_one(&pool).await.unwrap();
    sqlx::query("INSERT INTO auth_identities (user_id, provider, provider_uid, email, password_hash) VALUES ($1, 'email', 'demo@demo.com', 'demo@demo.com', $2)
         ON CONFLICT (provider, provider_uid) DO UPDATE SET password_hash = EXCLUDED.password_hash").bind(uid).bind(&hash).execute(&pool).await.unwrap();
    let (s, h, v) = common::call(
        &pool,
        "POST",
        "/auth/login",
        &[],
        Some(json!({ "email": "demo@demo.com", "password": "demo123" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(h["set-cookie"].to_str().unwrap().starts_with("ws_session="));
    common::assert_all_reconciled(&pool).await;
}
