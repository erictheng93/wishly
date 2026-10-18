//! 密碼登入（註冊 / 登入 / 重設）與 Google OIDC。Session 簽發、OTP、identity 連結都沿用 auth.rs。
use crate::{
    auth::{consume_otp, env, issue_otp, norm_email, session_response, upsert_identity_in_tx},
    error::AppError,
    AppState,
};
use argon2::{
    password_hash::{rand_core::OsRng, SaltString},
    Argon2, PasswordHash, PasswordHasher, PasswordVerifier,
};
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::Response,
    routing::post,
    Json, Router,
};
use jsonwebtoken::{decode, decode_header, jwk::JwkSet, Algorithm, DecodingKey, Validation};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::LazyLock;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/auth/register", post(register))
        .route("/auth/register/verify", post(register_verify))
        .route("/auth/login", post(login))
        .route("/auth/password/reset/request", post(reset_request))
        .route("/auth/password/reset/confirm", post(reset_confirm))
}

// ---------- 密碼 ----------
pub(crate) fn hash_pw(p: &str) -> Result<String, AppError> {
    Argon2::default()
        .hash_password(p.as_bytes(), &SaltString::generate(&mut OsRng))
        .map(|h| h.to_string())
        .map_err(|e| {
            tracing::error!(error = %e, "argon2 hash");
            AppError::problem(500, "INTERNAL_ERROR", "系統錯誤")
        })
}
pub(crate) fn verify_pw(p: &str, hash: &str) -> bool {
    PasswordHash::new(hash)
        .is_ok_and(|h| Argon2::default().verify_password(p.as_bytes(), &h).is_ok())
}
/// 帳號不存在時也驗一次，讓回應時間與「密碼錯誤」相近。
static DUMMY_HASH: LazyLock<String> =
    LazyLock::new(|| hash_pw("dummy-password-for-timing").unwrap());

pub(crate) fn check_pw(p: &str) -> Result<(), AppError> {
    if (8..=128).contains(&p.chars().count()) {
        Ok(())
    } else {
        Err(AppError::invalid(
            "/password",
            "RANGE",
            "密碼長度須為 8–128 字元",
        ))
    }
}
pub(crate) async fn hash_blocking(p: String) -> Result<String, AppError> {
    tokio::task::spawn_blocking(move || hash_pw(&p))
        .await
        .map_err(|_| AppError::problem(500, "INTERNAL_ERROR", "系統錯誤"))?
}

#[derive(Deserialize)]
struct RegisterReq {
    email: String,
    password: String,
    display_name: String,
}

async fn register(
    State(st): State<AppState>,
    crate::ratelimit::Peer(ip): crate::ratelimit::Peer,
    Json(r): Json<RegisterReq>,
) -> Result<(StatusCode, Json<Value>), AppError> {
    crate::ratelimit::check(&st.pool, &format!("otp_req:{ip}"), 20, 3600).await?;
    let email = norm_email(&r.email)?;
    check_pw(&r.password)?;
    let name = r.display_name.trim();
    if !(1..=50).contains(&name.chars().count()) {
        return Err(AppError::invalid(
            "/display_name",
            "RANGE",
            "暱稱長度須為 1–50 字",
        ));
    }
    // 不論 email 是否已註冊都回 202（不洩漏帳號存在）；已有密碼者在 verify 時才拒絕
    let h = hash_blocking(r.password).await?;
    issue_otp(&st.pool, &email, "register", Some(&h), Some(name)).await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({ "expires_in": 600, "resend_after": 60 })),
    ))
}

#[derive(Deserialize)]
struct CodeReq {
    email: String,
    code: String,
}

async fn register_verify(
    State(st): State<AppState>,
    crate::ratelimit::Peer(ip): crate::ratelimit::Peer,
    headers: HeaderMap,
    Json(r): Json<CodeReq>,
) -> Result<Response, AppError> {
    crate::ratelimit::check(&st.pool, &format!("otp_ver:{ip}"), 30, 3600).await?;
    let email = norm_email(&r.email)?;
    let (pw, name) = consume_otp(&st.pool, &email, &r.code, "register").await?;
    let (Some(pw), Some(name)) = (pw, name) else {
        return Err(crate::auth::otp_invalid());
    };
    let mut tx = st.pool.begin().await?;
    let (uid, is_new) =
        upsert_identity_in_tx(&mut tx, "email", &email, Some(&email), &name).await?;
    let updated = sqlx::query("UPDATE auth_identities SET password_hash=$2 WHERE provider='email' AND provider_uid=$1 AND password_hash IS NULL")
        .bind(&email).bind(pw).execute(&mut *tx).await?;
    if updated.rows_affected() != 1 {
        return Err(AppError::problem(
            409,
            "EMAIL_EXISTS",
            "此 Email 已註冊，請直接登入或使用忘記密碼。",
        ));
    }
    tx.commit().await?;
    session_response(&st.pool, &headers, &ip, "register", uid, is_new).await
}

fn bad_credentials() -> AppError {
    AppError::problem(401, "INVALID_CREDENTIALS", "Email 或密碼錯誤")
}

#[derive(Deserialize)]
struct LoginReq {
    email: String,
    password: String,
}

// IP 見 config::client_ip（TRUSTED_PROXY）。15 分鐘內失敗：同 (email, IP) 5 次即鎖（主要）；
// 同 email 跨 IP 合計 20 次即鎖（防分散式暴力的上限）；同 IP 30 次即鎖。
// 取捨：攻擊者單一 IP 只能鎖自己那組 (email, IP)，無法鎖死受害者在別的 IP 的登入；
// 要鎖死他人需 20 次分散失敗（成本較高），且受害者重設密碼 / 成功登入即清除該 email 的紀錄。
async fn login(
    State(st): State<AppState>,
    parts: axum::http::request::Parts,
    Json(r): Json<LoginReq>,
) -> Result<Response, AppError> {
    let email = r.email.trim().to_lowercase();
    let headers = parts.headers.clone();
    let ip = crate::config::client_ip(&parts, &crate::config::get());
    let (by_pair, by_email, by_ip): (i64, i64, i64) = sqlx::query_as(
        "SELECT count(*) FILTER (WHERE email=$1 AND ip=$2), count(*) FILTER (WHERE email=$1), count(*) FILTER (WHERE ip=$2) FROM login_failures
         WHERE created_at > now() - interval '15 minutes' AND (email=$1 OR ip=$2)").bind(&email).bind(&ip).fetch_one(&st.pool).await?;
    if by_pair >= 5 || by_email >= 20 || by_ip >= 30 {
        return Err(crate::auth::rate_limited(300));
    }
    let row: Option<(uuid::Uuid, Option<String>)> = sqlx::query_as(
        "SELECT i.user_id, i.password_hash FROM auth_identities i JOIN users u ON u.id=i.user_id
         WHERE i.provider='email' AND i.provider_uid=$1 AND u.deleted_at IS NULL",
    )
    .bind(&email)
    .fetch_optional(&st.pool)
    .await?;
    let (uid, hash) = match row {
        Some((u, h)) => (Some(u), h),
        None => (None, None),
    };
    let pw = r.password;
    let ok = tokio::task::spawn_blocking(move || {
        let real = hash.as_deref();
        let good = verify_pw(&pw, real.unwrap_or(&DUMMY_HASH)); // 無帳號 / 無密碼也跑一次
        good && real.is_some() && pw.chars().count() <= 128
    })
    .await
    .unwrap_or(false);
    let Some(uid) = uid.filter(|_| ok) else {
        sqlx::query("INSERT INTO login_failures (email, ip) VALUES ($1,$2)")
            .bind(&email)
            .bind(&ip)
            .execute(&st.pool)
            .await?;
        return Err(bad_credentials());
    };
    sqlx::query("DELETE FROM login_failures WHERE email=$1")
        .bind(&email)
        .execute(&st.pool)
        .await?;
    session_response(&st.pool, &headers, &ip, "password", uid, false).await
}

#[derive(Deserialize)]
struct ResetReq {
    email: String,
}

async fn reset_request(
    State(st): State<AppState>,
    crate::ratelimit::Peer(ip): crate::ratelimit::Peer,
    Json(r): Json<ResetReq>,
) -> Result<(StatusCode, Json<Value>), AppError> {
    crate::ratelimit::check(&st.pool, &format!("otp_req:{ip}"), 20, 3600).await?;
    let email = norm_email(&r.email)?;
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM users WHERE email=$1 AND deleted_at IS NULL)",
    )
    .bind(&email)
    .fetch_one(&st.pool)
    .await?;
    if exists {
        match issue_otp(&st.pool, &email, "reset", None, None).await {
            Ok(()) | Err(AppError::Problem { status: 429, .. }) => {} // 限流也回 202，維持不洩漏
            Err(e) => return Err(e),
        }
    }
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({ "expires_in": 600, "resend_after": 60 })),
    ))
}

#[derive(Deserialize)]
struct ResetConfirm {
    email: String,
    code: String,
    new_password: String,
}

async fn reset_confirm(
    State(st): State<AppState>,
    crate::ratelimit::Peer(ip): crate::ratelimit::Peer,
    Json(r): Json<ResetConfirm>,
) -> Result<StatusCode, AppError> {
    crate::ratelimit::check(&st.pool, &format!("otp_ver:{ip}"), 30, 3600).await?;
    let email = norm_email(&r.email)?;
    check_pw(&r.new_password)?;
    consume_otp(&st.pool, &email, &r.code, "reset").await?;
    let uid: uuid::Uuid =
        sqlx::query_scalar("SELECT id FROM users WHERE email=$1 AND deleted_at IS NULL")
            .bind(&email)
            .fetch_optional(&st.pool)
            .await?
            .ok_or_else(|| {
                AppError::problem(400, "OTP_INVALID", "驗證碼錯誤或已過期，請重新取得。")
            })?;
    let h = hash_blocking(r.new_password).await?;
    let mut tx = st.pool.begin().await?;
    crate::identities::lock_identity(&mut tx, "email", &email, Some(&email)).await?;
    // 雜湊期間可能已解除／更換 email；鎖住原帳號並重新核對，不得改到新持有者。
    let blocked: bool = sqlx::query_scalar("SELECT blocked_at IS NOT NULL FROM users WHERE id=$1 AND email=$2 AND deleted_at IS NULL FOR UPDATE")
        .bind(uid).bind(&email).fetch_optional(&mut *tx).await?.ok_or_else(crate::auth::otp_invalid)?;
    if blocked {
        return Err(AppError::problem(
            403,
            "ACCOUNT_BLOCKED",
            "帳號已停權，請聯絡客服",
        ));
    }
    let identity: Option<uuid::Uuid> = sqlx::query_scalar(
        "SELECT user_id FROM auth_identities WHERE provider='email' AND provider_uid=$1",
    )
    .bind(&email)
    .fetch_optional(&mut *tx)
    .await?;
    if identity.is_some_and(|id| id != uid) {
        return Err(crate::identities::in_use());
    }
    sqlx::query("INSERT INTO auth_identities (user_id,provider,provider_uid,email,password_hash) VALUES ($1,'email',$2,$2,$3)
                 ON CONFLICT (provider,provider_uid) DO UPDATE SET password_hash=EXCLUDED.password_hash WHERE auth_identities.user_id=EXCLUDED.user_id")
        .bind(uid).bind(&email).bind(&h).execute(&mut *tx).await?;
    if identity.is_none() {
        crate::identities::notify(&mut tx, uid, "auth.identity_linked", "email", Some(&email))
            .await?;
    } else {
        crate::identities::notify(&mut tx, uid, "auth.password_changed", "email", Some(&email))
            .await?;
    }
    crate::security::audit(
        &mut *tx,
        uid,
        "auth.password_reset",
        "users",
        uid,
        crate::security::inet(&ip).as_deref(),
        json!({}),
    )
    .await?;
    sqlx::query("UPDATE sessions SET revoked_at = now() WHERE user_id=$1 AND revoked_at IS NULL")
        .bind(uid)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM login_failures WHERE email=$1")
        .bind(&email)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------- Google OIDC ----------
pub struct GoogleCfg {
    pub auth_url: String,
    token_url: String,
    jwks_url: String,
    issuers: Vec<String>,
    pub client_id: String,
    secret: String,
    pub redirect_uri: String,
}
/// 環境變數存在且非空才算已設定（空字串視為未設定）
pub(crate) fn nonempty(k: &str) -> Option<String> {
    std::env::var(k).ok().filter(|v| !v.trim().is_empty())
}

impl GoogleCfg {
    pub fn from_env() -> Option<GoogleCfg> {
        Some(GoogleCfg {
            client_id: nonempty("GOOGLE_CLIENT_ID")?,
            secret: nonempty("GOOGLE_CLIENT_SECRET")?,
            redirect_uri: env(
                "GOOGLE_REDIRECT_URI",
                "http://localhost:8080/api/v1/auth/oauth/google/callback",
            ),
            auth_url: env(
                "GOOGLE_AUTH_URL",
                "https://accounts.google.com/o/oauth2/v2/auth",
            ),
            token_url: env("GOOGLE_TOKEN_URL", "https://oauth2.googleapis.com/token"),
            jwks_url: env(
                "GOOGLE_JWKS_URL",
                "https://www.googleapis.com/oauth2/v3/certs",
            ),
            issuers: match std::env::var("GOOGLE_ISSUER") {
                Ok(i) if !i.is_empty() => vec![i],
                _ => vec![
                    "https://accounts.google.com".into(),
                    "accounts.google.com".into(),
                ],
            },
        })
    }
}
pub struct GoogleIdentity {
    pub sub: String,
    pub email: Option<String>,
    pub name: String,
    pub picture: Option<String>,
}

struct CachedJwks {
    url: String,
    keys: JwkSet,
    expires: std::time::Instant,
    unknown_refresh: Option<std::time::Instant>,
}
static JWKS: LazyLock<tokio::sync::Mutex<Option<CachedJwks>>> =
    LazyLock::new(|| tokio::sync::Mutex::new(None));

fn key_from(keys: &JwkSet, kid: &str) -> Option<DecodingKey> {
    keys.find(kid).and_then(|k| DecodingKey::from_jwk(k).ok())
}

async fn google_key(
    client: &reqwest::Client,
    url: &str,
    kid: &str,
) -> Result<Option<DecodingKey>, reqwest::Error> {
    // 單一 Google 端點共用抓取；未知 kid 至多每 30 秒觸發一次額外更新，防止偽造 kid 打爆上游。
    let mut cache = JWKS.lock().await;
    let now = std::time::Instant::now();
    let mut unknown_refresh = None;
    if let Some(cached) = cache.as_ref().filter(|c| c.url == url && c.expires > now) {
        if let Some(key) = key_from(&cached.keys, kid) {
            return Ok(Some(key));
        }
        if cached
            .unknown_refresh
            .is_some_and(|t| now.duration_since(t).as_secs() < 30)
        {
            return Ok(None);
        }
        unknown_refresh = Some(now);
    }
    let response = client.get(url).send().await?.error_for_status()?;
    let directives = response
        .headers()
        .get(reqwest::header::CACHE_CONTROL)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    let directives: Vec<_> = directives.split(',').map(str::trim).collect();
    let ttl = if directives
        .iter()
        .any(|d| d.eq_ignore_ascii_case("no-store") || d.eq_ignore_ascii_case("no-cache"))
    {
        0
    } else {
        directives
            .iter()
            .find_map(|d| {
                let (name, value) = d.split_once('=')?;
                name.trim()
                    .eq_ignore_ascii_case("max-age")
                    .then(|| value.trim().trim_matches('"').parse::<u64>().ok())
                    .flatten()
            })
            .unwrap_or(300)
            .min(86400)
    };
    let age = response
        .headers()
        .get(reqwest::header::AGE)
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);
    let keys: JwkSet = response.json().await?;
    let key = key_from(&keys, kid);
    *cache = Some(CachedJwks {
        url: url.into(),
        keys,
        expires: now + std::time::Duration::from_secs(ttl.saturating_sub(age)),
        unknown_refresh,
    });
    Ok(key)
}

/// code 換 token → 驗 id_token。Ok(None) = id_token 不合法；Err = 供應商端點故障（502）。
pub async fn google_identity(
    code: &str,
    verifier: &str,
) -> Result<Option<GoogleIdentity>, AppError> {
    let cfg = GoogleCfg::from_env()
        .ok_or_else(|| AppError::problem(502, "OAUTH_PROVIDER_ERROR", "Google 登入尚未設定"))?;
    let bad = |e: String| {
        tracing::error!(error = %e, "google oauth");
        AppError::problem(502, "OAUTH_PROVIDER_ERROR", "Google 登入暫時無法使用")
    };
    let c = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| bad(e.to_string()))?;
    let tok: Value = c
        .post(&cfg.token_url)
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", &cfg.redirect_uri),
            ("client_id", &cfg.client_id),
            ("client_secret", &cfg.secret),
            ("code_verifier", verifier),
        ])
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| bad(e.to_string()))?
        .json()
        .await
        .map_err(|e| bad(e.to_string()))?;
    let idt = tok["id_token"]
        .as_str()
        .ok_or_else(|| bad("no id_token".into()))?;
    let Ok(Some(kid)) = decode_header(idt).map(|h| h.kid) else {
        return Ok(None);
    };
    let Some(key) = google_key(&c, &cfg.jwks_url, &kid)
        .await
        .map_err(|e| bad(e.to_string()))?
    else {
        return Ok(None);
    };
    let mut v = Validation::new(Algorithm::RS256);
    v.set_audience(&[&cfg.client_id]);
    v.set_issuer(&cfg.issuers);
    v.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
    let Ok(d) = decode::<Value>(idt, &key, &v) else {
        return Ok(None);
    };
    let cl = d.claims;
    let Some(sub) = cl["sub"].as_str().filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    let verified = cl["email_verified"].as_bool() == Some(true)
        || cl["email_verified"].as_str() == Some("true");
    let email = cl["email"]
        .as_str()
        .filter(|_| verified)
        .and_then(|e| norm_email(e).ok());
    let name = cl["name"]
        .as_str()
        .or_else(|| email.as_deref().and_then(|e| e.split('@').next()))
        .unwrap_or("Google 使用者")
        .to_string();
    Ok(Some(GoogleIdentity {
        sub: sub.to_string(),
        email,
        name,
        picture: cl["picture"].as_str().map(String::from),
    }))
}
