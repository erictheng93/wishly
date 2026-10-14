//! 密碼登入與 Google OIDC 整合測試：mock token endpoint + JWKS，不依賴外部服務。
mod common;
use axum::{
    http::{header, StatusCode},
    routing::{get, post},
    Form, Json, Router,
};
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::{collections::HashMap, sync::OnceLock};
use uuid::Uuid;
use wishsync_api::{app, AppState};

static GOOGLE_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

const RSA_PEM: &str = "-----BEGIN PRIVATE KEY-----\nMIIEvAIBADANBgkqhkiG9w0BAQEFAASCBKYwggSiAgEAAoIBAQCwYdSsFyEqb2fM\nC+GAozSLr/6KbAI8valKrNdWYvixQ4ilybk9kcFMCIQaFvte11/z8rqVYOMoRX+K\nnHhG054PIGufQockZtUBScwVtXLqv7XMvcF4ubURT7AkB6eV1Qvvs1bRgcIQRxgF\nXRQiZehvXXtc8Fc9YpJul+v93siBlFLptqF7dbvI2gh6v9rjdlV+g3robDX4flUw\nngWnakF+rsrFCeWwRyK7Mvjd+8L47RtRlgU/IiNdu1kt2Yg3RVecEmYOCdUue0i2\nbUFxFw8sZk97ydJBXhBCpdE9g4CgPTKCwbrjB1ncODjxqnbpVwMLsEtvPcjmOhD0\n4xOI8uSJAgMBAAECggEAFNf+0rZqBjKDpOabh8DhrwdFkJSagBSWurspuGz6QDJ/\npb3svOSqX+Kav92K56aSkYjT+V6pgYMTAy2h/ha4dUeqyDeLsmbPkbcnncIjWi/9\nGzyqbiqeBfVxlpRr/sYeQr2iCyCnLxbDtTCOdlEuMmjs8Or7GEc7kEO2UPpAniVX\ng9W3I14qxbF9S25BK2T9mLS5IKCLzR2w9fIzWEV19NlDqp+lUW9W05TyYBXTS6S3\n8BRTcVwfzlJHUF4InFnsjokl9Txji+fqqapkz33l6VkSMSzGg2ZvBr3Oe0hIN2UN\nvc/W+w6lrozWYO+CPMZQ28ldOtKEkvePS9cuNdIJnQKBgQDiLJ+uF8/ye9wOHgiA\nJpDQwGna7uh/Am4qb5PIpV4S4iQ7ZL6dl3jrWLdC+kHh6Pl4Cz9yZbez52QRy9kw\naiE+0yN6z94LHyXmNn4lW9kYqnUIuRp8AdEM3E4+rzMGunKoniv7/mOeAG2wC4cH\nZPYars8WW7Hdhff0ZCT7oVe0ZQKBgQDHpEhko9aBFtcGAiDacqaIfvz69Etd/T6Z\nFgBrc2HC5IboMpxIso3VckY6wGzj+vyAhCZhFFEihWcdpYEEZmXSHYnDfX4HN8CY\nYmXXwSkgCnePQfq8kkWXxrwmgW5MvTdGksOERiveqJjdPtPonraMQK0+sGhQksUE\nDOekMmaTVQKBgFVepJMQ1+PeDoyhl3HPnL++sPX7UXSHVU/dN7n3eU/FXgmxGCXh\nw7mJjfrQ/UcpKei+zh7+990HDQWOnRciKBRPm8fCaDxocGS50tmFFqexx2UYgT+n\na+Hf0gTlGmyCub6dnVqLhcxguwZFA52t5RrjOrVkvPkxlQsBNdho6PLlAoGAFXLX\n+EAF7q6GgXW/E9kXfTivc0LFyh5IGGhduS2XRjoJKG8vjTvpsxHo66z6xe+UKzaH\nbQyivuO4NV4vo0phDBbyUxdrb9kUjwqbSHfPNEsLl0+OYa4RXgIS+swTDpmRk7p2\n2QI0+mIAslxqpeZPVHgqZL7NJkLuecXtKR3lGVUCgYAL8bhPlRruNgEUWCl+gPkO\nYbZmmuFY/4uTzAi2ZEOWigsIkABXHkaZKKtgXvfPb7OsfOIM126Abxs3pFga52ls\npeo77uySdh4QwKJ8V2oV3PdWgC/vl+VoxOuEEsRplcXp+p73pJ+wZhlCqj+cr8ZR\nDIApeLQ2Yx11IDSVjBRReA==\n-----END PRIVATE KEY-----\n";
const RSA_N: &str = "sGHUrBchKm9nzAvhgKM0i6_-imwCPL2pSqzXVmL4sUOIpcm5PZHBTAiEGhb7Xtdf8_K6lWDjKEV_ipx4RtOeDyBrn0KHJGbVAUnMFbVy6r-1zL3BeLm1EU-wJAenldUL77NW0YHCEEcYBV0UImXob117XPBXPWKSbpfr_d7IgZRS6bahe3W7yNoIer_a43ZVfoN66Gw1-H5VMJ4Fp2pBfq7KxQnlsEciuzL43fvC-O0bUZYFPyIjXbtZLdmIN0VXnBJmDgnVLntItm1BcRcPLGZPe8nSQV4QQqXRPYOAoD0ygsG64wdZ3Dg48ap26VcDC7BLbz3I5joQ9OMTiPLkiQ";

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

async fn recover_code(pool: &PgPool, email: &str, purpose: &str) -> String {
    let h: Vec<u8> = sqlx::query_scalar("SELECT code_hash FROM otp_challenges WHERE email=$1 AND purpose=$2 ORDER BY created_at DESC LIMIT 1")
        .bind(email).bind(purpose).fetch_one(pool).await.unwrap();
    (0..1_000_000u32)
        .map(|n| format!("{n:06}"))
        .find(|c| Sha256::digest(format!("{c}dev-pepper").as_bytes()).as_slice() == h.as_slice())
        .unwrap()
}
fn session_cookie(h: &axum::http::HeaderMap) -> String {
    h.get_all(header::SET_COOKIE)
        .iter()
        .map(|v| v.to_str().unwrap())
        .find(|c| c.starts_with("ws_session="))
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string()
}
async fn age_otps(pool: &PgPool) {
    sqlx::query("UPDATE otp_challenges SET created_at = created_at - interval '2 minutes'")
        .execute(pool)
        .await
        .unwrap();
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn password_register_login_reset(pool: PgPool) {
    let app = app(AppState { pool: pool.clone() });
    let email = format!("pw-{}@example.com", Uuid::new_v4());
    let reg = |pw: &str| json!({ "email": email, "password": pw, "display_name": "小明" });
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/v1/auth/register",
            None,
            Some(reg("short"))
        )
        .await
        .0,
        422
    );
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/v1/auth/register",
            None,
            Some(reg(&"x".repeat(129)))
        )
        .await
        .0,
        422
    );
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/v1/auth/register",
            None,
            Some(reg("correct-horse"))
        )
        .await
        .0,
        202
    );
    // 驗證前不存在 user，也不能登入
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM users WHERE email=$1")
        .bind(&email)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 0);
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/v1/auth/login",
            None,
            Some(json!({ "email": email, "password": "correct-horse" }))
        )
        .await
        .0,
        401
    );
    // login 用途的 OTP 不能拿來驗註冊
    let code = recover_code(&pool, &email, "register").await;
    let wrong = if code == "000000" { "000001" } else { "000000" };
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/v1/auth/register/verify",
            None,
            Some(json!({ "email": email, "code": wrong }))
        )
        .await
        .0,
        400
    );
    let (st, h, b) = call(
        &app,
        "POST",
        "/api/v1/auth/register/verify",
        None,
        Some(json!({ "email": email, "code": code })),
    )
    .await;
    assert_eq!(st, 200, "{b}");
    assert_eq!(b["is_new_user"], true);
    assert_eq!(b["user"]["display_name"], "小明");
    assert!(h
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap()
        .contains("HttpOnly"));
    let hash: String =
        sqlx::query_scalar("SELECT password_hash FROM auth_identities WHERE provider='email'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(hash.starts_with("$argon2id$"));
    // 登入
    let (st, h, b) = call(
        &app,
        "POST",
        "/api/v1/auth/login",
        None,
        Some(json!({ "email": email.to_uppercase(), "password": "correct-horse" })),
    )
    .await;
    assert_eq!(
        (st, b["is_new_user"].clone()),
        (StatusCode::OK, json!(false))
    );
    let old = session_cookie(&h);
    assert_eq!(
        call(&app, "GET", "/api/v1/me", Some(&old), None).await.0,
        200
    );
    // 錯誤密碼與不存在帳號：同一個 401 與相同內容
    let (s1, _, e1) = call(
        &app,
        "POST",
        "/api/v1/auth/login",
        None,
        Some(json!({ "email": email, "password": "wrong-password" })),
    )
    .await;
    let (s2, _, e2) = call(
        &app,
        "POST",
        "/api/v1/auth/login",
        None,
        Some(json!({ "email": "nobody@example.com", "password": "wrong-password" })),
    )
    .await;
    assert_eq!(
        (s1, s2),
        (StatusCode::UNAUTHORIZED, StatusCode::UNAUTHORIZED)
    );
    assert_eq!(e1, e2);
    assert_eq!(e1["code"], "INVALID_CREDENTIALS");
    // 已有密碼者再註冊 → 驗證時 409
    // 剛驗證成功（驗證碼已使用）不擋下一次請求；但新寄出、尚未使用的驗證碼 60 秒內不能再要
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/v1/auth/register",
            None,
            Some(reg("another-pass"))
        )
        .await
        .0,
        202
    );
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/v1/auth/register",
            None,
            Some(reg("another-pass"))
        )
        .await
        .0,
        429
    );
    age_otps(&pool).await;
    let code = recover_code(&pool, &email, "register").await;
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/v1/auth/register/verify",
            None,
            Some(json!({ "email": email, "code": code }))
        )
        .await
        .0,
        409
    );
    // 重設：不存在的 email 也 202（且不產生挑戰）
    age_otps(&pool).await;
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/v1/auth/password/reset/request",
            None,
            Some(json!({ "email": "ghost@example.com" }))
        )
        .await
        .0,
        202
    );
    let n: i64 =
        sqlx::query_scalar("SELECT count(*) FROM otp_challenges WHERE email='ghost@example.com'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(n, 0);
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/v1/auth/password/reset/request",
            None,
            Some(json!({ "email": email }))
        )
        .await
        .0,
        202
    );
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/v1/auth/password/reset/request",
            None,
            Some(json!({ "email": email }))
        )
        .await
        .0,
        202
    ); // 限流也 202
    let code = recover_code(&pool, &email, "reset").await;
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/v1/auth/password/reset/confirm",
            None,
            Some(json!({ "email": email, "code": code, "new_password": "tiny" }))
        )
        .await
        .0,
        422
    );
    let (st, ..) = call(
        &app,
        "POST",
        "/api/v1/auth/password/reset/confirm",
        None,
        Some(json!({ "email": email, "code": code, "new_password": "brand-new-pass" })),
    )
    .await;
    assert_eq!(st, 204);
    assert_eq!(
        call(&app, "GET", "/api/v1/me", Some(&old), None).await.0,
        401
    ); // 舊 session 失效
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/v1/auth/login",
            None,
            Some(json!({ "email": email, "password": "correct-horse" }))
        )
        .await
        .0,
        401
    );
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/v1/auth/login",
            None,
            Some(json!({ "email": email, "password": "brand-new-pass" }))
        )
        .await
        .0,
        200
    );
    // 失敗過多 → 429（每 email）
    for _ in 0..5 {
        call(
            &app,
            "POST",
            "/api/v1/auth/login",
            None,
            Some(json!({ "email": "brute@example.com", "password": "bad-password" })),
        )
        .await;
    }
    let (st, h, b) = call(
        &app,
        "POST",
        "/api/v1/auth/login",
        None,
        Some(json!({ "email": "brute@example.com", "password": "bad-password" })),
    )
    .await;
    assert_eq!(
        (st, b["code"].as_str()),
        (StatusCode::TOO_MANY_REQUESTS, Some("RATE_LIMITED"))
    );
    assert!(h.contains_key("retry-after"));
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn otp_account_sets_password_via_reset(pool: PgPool) {
    let app = app(AppState { pool: pool.clone() });
    let email = format!("otp-{}@example.com", Uuid::new_v4());
    call(
        &app,
        "POST",
        "/api/v1/auth/otp/request",
        None,
        Some(json!({ "email": email })),
    )
    .await;
    let code = recover_code(&pool, &email, "login").await;
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/v1/auth/otp/verify",
            None,
            Some(json!({ "email": email, "code": code }))
        )
        .await
        .0,
        200
    );
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/v1/auth/login",
            None,
            Some(json!({ "email": email, "password": "whatever-123" }))
        )
        .await
        .0,
        401
    );
    age_otps(&pool).await;
    call(
        &app,
        "POST",
        "/api/v1/auth/password/reset/request",
        None,
        Some(json!({ "email": email })),
    )
    .await;
    let code = recover_code(&pool, &email, "reset").await;
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/v1/auth/password/reset/confirm",
            None,
            Some(json!({ "email": email, "code": code, "new_password": "whatever-123" }))
        )
        .await
        .0,
        204
    );
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/v1/auth/login",
            None,
            Some(json!({ "email": email, "password": "whatever-123" }))
        )
        .await
        .0,
        200
    );
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM users WHERE email=$1")
        .bind(&email)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 1);
}

// ---------- Google ----------
/// 全程共用一個 mock（獨立執行緒 + runtime，不隨單一測試結束）：/token 把 code 當 id_token 原樣回傳，/jwks 提供測試公鑰。
fn init_mock() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            tokio::runtime::Runtime::new().unwrap().block_on(async {
                let mock = Router::new()
                    .route("/token", post(|Form(f): Form<HashMap<String, String>>| async move {
                        assert_eq!(f["client_id"], "gid"); assert_eq!(f["client_secret"], "gsec"); assert!(f.contains_key("code_verifier"));
                        Json(json!({ "id_token": f["code"] }))
                    }))
                    .route("/jwks", get(|| async { Json(json!({ "keys": [{ "kty": "RSA", "kid": "k1", "alg": "RS256", "use": "sig", "n": RSA_N, "e": "AQAB" }] })) }));
                let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                tx.send(format!("http://{}", l.local_addr().unwrap())).unwrap();
                axum::serve(l, mock).await.unwrap();
            })
        });
        let base = rx.recv().unwrap();
        std::env::set_var("GOOGLE_CLIENT_ID", "gid"); std::env::set_var("GOOGLE_CLIENT_SECRET", "gsec");
        std::env::set_var("GOOGLE_TOKEN_URL", format!("{base}/token")); std::env::set_var("GOOGLE_JWKS_URL", format!("{base}/jwks"));
    });
}

fn id_token(sub: &str, email: &str, verified: bool, aud: &str, exp_offset: i64) -> String {
    let mut h = Header::new(Algorithm::RS256);
    h.kid = Some("k1".into());
    let exp = chrono::Utc::now().timestamp() + exp_offset;
    let claims = json!({ "iss": "https://accounts.google.com", "aud": aud, "exp": exp, "sub": sub, "email": email, "email_verified": verified, "name": "G User", "picture": "https://img.example/g.png" });
    encode(
        &h,
        &claims,
        &EncodingKey::from_rsa_pem(RSA_PEM.as_bytes()).unwrap(),
    )
    .unwrap()
}

/// 走完 start → callback，回傳 (callback status, headers)
async fn google_login(app: &Router, idt: &str) -> (StatusCode, axum::http::HeaderMap) {
    let (st, h, _) = call(
        app,
        "GET",
        "/api/v1/auth/oauth/google/start?redirect=/lists",
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
        loc.starts_with("https://accounts.google.com/o/oauth2/v2/auth?")
            && loc.contains("scope=openid")
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
    let (st, h, _) = call(
        app,
        "GET",
        &format!("/api/v1/auth/oauth/google/callback?code={idt}&state={state}"),
        Some(&oc),
        None,
    )
    .await;
    (st, h)
}
fn failed(h: &axum::http::HeaderMap) -> bool {
    h.get(header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap()
        .ends_with("/login?error=oauth_failed")
        && !h.get_all(header::SET_COOKIE).iter().any(|c| {
            c.to_str().unwrap().starts_with("ws_session=")
                && !c.to_str().unwrap().contains("Max-Age=0")
        })
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn google_login_links_by_verified_email(pool: PgPool) {
    let _guard = GOOGLE_TEST_LOCK.lock().await;
    init_mock();
    let app = app(AppState { pool: pool.clone() });
    // 已有密碼帳號 → Google 同 email 連到同一 user
    let email = format!("g-{}@example.com", Uuid::new_v4());
    let uid: Uuid = sqlx::query_scalar(
        "INSERT INTO users (display_name, email) VALUES ('既有', $1) RETURNING id",
    )
    .bind(&email)
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO auth_identities (user_id, provider, provider_uid, email) VALUES ($1,'email',$2,$2)").bind(uid).bind(&email).execute(&pool).await.unwrap();

    let (st, h) = google_login(&app, &id_token("g-sub-1", &email, true, "gid", 3600)).await;
    assert_eq!(st, 302);
    assert!(h
        .get(header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap()
        .ends_with("/lists"));
    let cookie = session_cookie(&h);
    let (_, _, me) = call(&app, "GET", "/api/v1/me", Some(&cookie), None).await;
    assert_eq!(me["id"], uid.to_string());
    assert_eq!(me["display_name"], "既有");
    let provs: Vec<&str> = me["identities"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["provider"].as_str().unwrap())
        .collect();
    assert_eq!(provs, ["email", "google"]);
    // 再登入一次不重複建立
    assert_eq!(
        google_login(&app, &id_token("g-sub-1", &email, true, "gid", 3600))
            .await
            .0,
        302
    );
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM auth_identities WHERE provider='google'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 1);

    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM notifications WHERE user_id=$1 AND kind='auth.identity_linked'",
    )
    .bind(uid)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(n, 1, "自動合併只寄一次安全通知");

    // 新 email → 新帳號
    let new_email = format!("gnew-{}@example.com", Uuid::new_v4());
    let (_, h) = google_login(&app, &id_token("g-sub-2", &new_email, true, "gid", 3600)).await;
    let (_, _, me) = call(&app, "GET", "/api/v1/me", Some(&session_cookie(&h)), None).await;
    assert_ne!(me["id"], uid.to_string());
    assert_eq!(me["email"], new_email);
    assert_eq!(me["display_name"], "G User");
    assert!(me["avatar_url"].is_null(), "不合法來源的外部頭像不直接公開");
    let avatar: Option<String> = sqlx::query_scalar("SELECT avatar_key FROM users WHERE email=$1")
        .bind(&new_email)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(avatar.is_none(), "avatar_key 不再存外部 URL");

    // email_verified=false 不得連結到既有帳號（另建不帶 email 的新 user）
    let (_, h) = google_login(&app, &id_token("g-sub-3", &email, false, "gid", 3600)).await;
    let (_, _, me) = call(&app, "GET", "/api/v1/me", Some(&session_cookie(&h)), None).await;
    assert_ne!(me["id"], uid.to_string());
    assert!(me["email"].is_null());
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn google_rejects_bad_id_tokens(pool: PgPool) {
    let _guard = GOOGLE_TEST_LOCK.lock().await;
    init_mock();
    let app = app(AppState { pool: pool.clone() });
    for (name, t) in [
        (
            "wrong aud",
            id_token("s", "a@example.com", true, "other-client", 3600),
        ),
        (
            "expired",
            id_token("s", "a@example.com", true, "gid", -3600),
        ),
        ("garbage", "not.a.jwt".to_string()),
    ] {
        let (st, h) = google_login(&app, &t).await;
        assert_eq!(st, 302, "{name}");
        assert!(failed(&h), "{name}");
    }
    // 簽章被竄改
    let t = id_token("s", "a@example.com", true, "gid", 3600);
    let mut parts: Vec<String> = t.split('.').map(String::from).collect();
    parts[1] = parts[1].chars().rev().collect();
    assert!(failed(&google_login(&app, &parts.join(".")).await.1));
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM users")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 0);
    // state 不符
    let (_, h, _) = call(&app, "GET", "/api/v1/auth/oauth/google/start", None, None).await;
    let oc = h
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();
    let (st, h, _) = call(
        &app,
        "GET",
        "/api/v1/auth/oauth/google/callback?code=x&state=wrong",
        Some(&oc),
        None,
    )
    .await;
    assert_eq!(st, 302);
    assert!(failed(&h));
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn google_manual_link_binds_session_and_refuses_account_merge(pool: PgPool) {
    let _guard = GOOGLE_TEST_LOCK.lock().await;
    init_mock();
    let app = app(AppState { pool: pool.clone() });
    let uid: Uuid =
        sqlx::query_scalar("INSERT INTO users (display_name) VALUES ('LINE') RETURNING id")
            .fetch_one(&pool)
            .await
            .unwrap();
    sqlx::query("INSERT INTO auth_identities (user_id,provider,provider_uid) VALUES ($1,'line','manual-line')").bind(uid).execute(&pool).await.unwrap();
    let token = "manual-session";
    sqlx::query("INSERT INTO sessions (user_id,token_hash,expires_at) VALUES ($1,$2,now()+interval '1 day')")
        .bind(uid).bind(wishsync_api::session::hash_token(token)).execute(&pool).await.unwrap();
    let session = format!("ws_session={token}");
    assert_eq!(
        call(
            &app,
            "GET",
            "/api/v1/auth/oauth/google/start?link=true",
            None,
            None
        )
        .await
        .0,
        401
    );
    async fn start(app: &Router, session: &str) -> (String, String) {
        let (s, h, _) = call(
            app,
            "GET",
            "/api/v1/auth/oauth/google/start?link=true",
            Some(session),
            None,
        )
        .await;
        assert_eq!(s, 302);
        let url = reqwest::Url::parse(h[header::LOCATION].to_str().unwrap()).unwrap();
        let state = url
            .query_pairs()
            .find(|(k, _)| k == "state")
            .unwrap()
            .1
            .into_owned();
        let c = h[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string();
        (state, c)
    }
    let (state, oc) = start(&app, &session).await;
    let idt = id_token("manual-google", "manual@example.com", true, "gid", 3600);
    let uri = format!("/api/v1/auth/oauth/google/callback?state={state}&code={idt}");
    let (_, h, _) = call(&app, "GET", &uri, Some(&oc), None).await;
    assert!(h[header::LOCATION]
        .to_str()
        .unwrap()
        .ends_with("/settings?error=oauth_failed"));
    // 相同 user 的另一個 session 也不能完成這次綁定。
    sqlx::query("INSERT INTO sessions (user_id,token_hash,expires_at) VALUES ($1,$2,now()+interval '1 day')")
        .bind(uid).bind(wishsync_api::session::hash_token("other-session")).execute(&pool).await.unwrap();
    let (_, h, _) = call(
        &app,
        "GET",
        &uri,
        Some(&format!("{oc}; ws_session=other-session")),
        None,
    )
    .await;
    assert!(h[header::LOCATION]
        .to_str()
        .unwrap()
        .ends_with("/settings?error=oauth_failed"));
    let (_, h, _) = call(&app, "GET", &uri, Some(&format!("{oc}; {session}")), None).await;
    assert!(h[header::LOCATION]
        .to_str()
        .unwrap()
        .ends_with("/settings?linked=google"));
    assert!(
        !h.get_all(header::SET_COOKIE)
            .iter()
            .any(|v| v.to_str().unwrap().starts_with("ws_session=")),
        "綁定不切換登入 session"
    );
    let (_, _, me) = call(&app, "GET", "/api/v1/me", Some(&session), None).await;
    assert_eq!(me["id"], uid.to_string());
    assert_eq!(me["email"], "manual@example.com");
    let (state, oc) = start(&app, &session).await;
    let (_, h, _) = call(
        &app,
        "GET",
        &format!("/api/v1/auth/oauth/line/callback?state={state}&code=x"),
        Some(&format!("{oc}; {session}")),
        None,
    )
    .await;
    assert!(
        h[header::LOCATION]
            .to_str()
            .unwrap()
            .ends_with("/settings?error=oauth_failed"),
        "供應商不能交換"
    );
    // 另一 user 已持有的 Google identity 不得被搬走。
    let other: Uuid =
        sqlx::query_scalar("INSERT INTO users (display_name) VALUES ('other') RETURNING id")
            .fetch_one(&pool)
            .await
            .unwrap();
    sqlx::query("INSERT INTO auth_identities (user_id,provider,provider_uid) VALUES ($1,'google','occupied-google')").bind(other).execute(&pool).await.unwrap();
    let (state, oc) = start(&app, &session).await;
    let idt = id_token("occupied-google", "occupied@example.com", true, "gid", 3600);
    let (_, h, _) = call(
        &app,
        "GET",
        &format!("/api/v1/auth/oauth/google/callback?state={state}&code={idt}"),
        Some(&format!("{oc}; {session}")),
        None,
    )
    .await;
    assert!(h[header::LOCATION]
        .to_str()
        .unwrap()
        .ends_with("/settings?error=identity_in_use"));
    let owner: Uuid = sqlx::query_scalar(
        "SELECT user_id FROM auth_identities WHERE provider_uid='occupied-google'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(owner, other);
}

#[tokio::test]
async fn google_jwks_cache_respects_ttl_and_rotation() {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    let _guard = GOOGLE_TEST_LOCK.lock().await;
    init_mock();
    struct Restore(String);
    impl Drop for Restore {
        fn drop(&mut self) {
            std::env::set_var("GOOGLE_JWKS_URL", &self.0);
        }
    }
    let _restore = Restore(std::env::var("GOOGLE_JWKS_URL").unwrap());
    let calls = Arc::new(AtomicUsize::new(0));
    let kid = Arc::new(std::sync::Mutex::new("k1"));
    let ttl = Arc::new(AtomicUsize::new(3600));
    let mock = Router::new().route("/keys", get({
        let (calls, kid, ttl) = (calls.clone(), kid.clone(), ttl.clone());
        move || { let (calls, kid, ttl) = (calls.clone(), kid.clone(), ttl.clone()); async move {
            calls.fetch_add(1, Ordering::SeqCst);
            ([(header::CACHE_CONTROL, format!("max-age={}", ttl.load(Ordering::SeqCst)))], Json(json!({"keys":[{"kty":"RSA","kid":*kid.lock().unwrap(),"alg":"RS256","use":"sig","n":RSA_N,"e":"AQAB"}]})))
        }}
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    std::env::set_var(
        "GOOGLE_JWKS_URL",
        format!("http://{}/keys", listener.local_addr().unwrap()),
    );
    let server = tokio::spawn(async { axum::serve(listener, mock).await.unwrap() });
    let t = id_token("cache", "cache@example.com", true, "gid", 3600);
    for _ in 0..2 {
        assert!(wishsync_api::auth_ext::google_identity(&t, "ver")
            .await
            .unwrap()
            .is_some());
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1, "有效期間共用 JWKS");
    *kid.lock().unwrap() = "k2";
    let mut h = Header::new(Algorithm::RS256);
    h.kid = Some("k2".into());
    let t = encode(&h, &json!({"iss":"https://accounts.google.com","aud":"gid","exp":chrono::Utc::now().timestamp()+3600,"sub":"cache"}), &EncodingKey::from_rsa_pem(RSA_PEM.as_bytes()).unwrap()).unwrap();
    ttl.store(0, Ordering::SeqCst);
    assert!(
        wishsync_api::auth_ext::google_identity(&t, "ver")
            .await
            .unwrap()
            .is_some(),
        "未知 kid 重新抓取，支援輪替"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert!(wishsync_api::auth_ext::google_identity(&t, "ver")
        .await
        .unwrap()
        .is_some());
    assert_eq!(calls.load(Ordering::SeqCst), 3, "max-age=0 不保留快取");
    server.abort();
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn google_login_redirects_blocked_and_ambiguous_accounts(pool: PgPool) {
    let _guard = GOOGLE_TEST_LOCK.lock().await;
    init_mock();
    let app = app(AppState { pool: pool.clone() });
    let blocked: Uuid = sqlx::query_scalar("INSERT INTO users (display_name,email,blocked_at) VALUES ('blocked','blocked@example.com',now()) RETURNING id").fetch_one(&pool).await.unwrap();
    sqlx::query("INSERT INTO auth_identities (user_id,provider,provider_uid,email) VALUES ($1,'google','blocked-google','blocked@example.com')").bind(blocked).execute(&pool).await.unwrap();
    let other: Uuid = sqlx::query_scalar(
        "INSERT INTO users (display_name,email) VALUES ('other','other@example.com') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO auth_identities (user_id,provider,provider_uid,email) VALUES ($1,'line','other-line','blocked@example.com')").bind(other).execute(&pool).await.unwrap();
    for (sub, expected) in [
        ("blocked-google", "account_blocked"),
        ("new-google", "identity_in_use"),
    ] {
        let (status, headers) = google_login(
            &app,
            &id_token(sub, "blocked@example.com", true, "gid", 3600),
        )
        .await;
        assert_eq!(status, StatusCode::FOUND);
        assert!(headers[header::LOCATION]
            .to_str()
            .unwrap()
            .ends_with(&format!("/login?error={expected}")));
        assert!(headers.get_all(header::SET_COOKIE).iter().any(|v| v
            .to_str()
            .unwrap()
            .starts_with("ws_oauth_state=; Max-Age=0")));
        assert!(!headers.get_all(header::SET_COOKIE).iter().any(|v| v
            .to_str()
            .unwrap()
            .starts_with("ws_session=")
            && !v.to_str().unwrap().contains("Max-Age=0")));
    }
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM sessions")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
}
