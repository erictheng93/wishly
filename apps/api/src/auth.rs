//! Auth：Email OTP、LINE OAuth2（code flow + PKCE）、登出、GET/PATCH /me。簽發 session（DB 只存 SHA-256(token)）。
use crate::{
    error::AppError,
    ratelimit::Peer,
    session::{hash_token, CurrentUser},
    AppState,
};
use axum::{
    extract::{FromRequestParts, Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{AppendHeaders, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD as B64, Engine};
use hmac::{Hmac, Mac};
use rand::{Rng, RngCore};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

const SESSION_DAYS: i64 = 30;
const OTP_TTL_SECS: i64 = 600;
const OTP_MAX_ATTEMPTS: i16 = 5;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/auth/otp/request", post(otp_request))
        .route("/auth/otp/verify", post(otp_verify))
        .route("/auth/oauth/{provider}/start", get(oauth_start))
        .route("/auth/oauth/{provider}/callback", get(oauth_callback))
        .route("/auth/native/exchange", post(native_exchange))
        .route("/auth/native/link-confirm", post(native_link_confirm))
        .route("/auth/oauth/{provider}/link-ticket", post(link_ticket))
        .route("/auth/logout", post(logout))
        .merge(crate::auth_ext::routes())
        .merge(crate::identities::routes())
        .route("/me", get(me).patch(patch_me))
}

pub(crate) fn env(k: &str, d: &str) -> String {
    std::env::var(k).unwrap_or_else(|_| d.to_string())
}
fn pepper() -> String {
    crate::config::get().otp_pepper
}
pub(crate) fn app_url() -> String {
    crate::config::get().app_url
}
pub(crate) fn secure() -> &'static str {
    crate::config::get().secure_cookie()
}

pub fn code_hash(code: &str, pepper: &str) -> Vec<u8> {
    Sha256::digest(format!("{code}{pepper}").as_bytes()).to_vec()
}

pub(crate) fn cookie(h: &HeaderMap, name: &str) -> Option<String> {
    h.get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .find_map(|c| c.trim().strip_prefix(name)?.strip_prefix('='))
        .map(String::from)
}
pub(crate) fn session_cookie(token: &str) -> String {
    format!(
        "ws_session={token}; Path=/; Max-Age={}; HttpOnly; SameSite=Lax{}",
        SESSION_DAYS * 86400,
        secure()
    )
}

// ---------- session / identity ----------
/// 簽發 session，同交易寫 audit `auth.login`（method：otp / password / register / line / google / native）。
pub(crate) async fn new_session(
    pool: &PgPool,
    user_id: Uuid,
    ua: Option<&str>,
    ip: &str,
    method: &str,
) -> Result<String, AppError> {
    let mut tx = pool.begin().await?;
    let blocked: bool = sqlx::query_scalar(
        "SELECT blocked_at IS NOT NULL FROM users WHERE id=$1 AND deleted_at IS NULL FOR UPDATE",
    )
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(AppError::Unauthorized)?;
    if blocked {
        return Err(AppError::problem(
            403,
            "ACCOUNT_BLOCKED",
            "帳號已停權，請聯絡客服",
        ));
    }
    let mut b = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut b);
    let token = B64.encode(b);
    let ip = crate::security::inet(ip);
    let sid: Uuid = sqlx::query_scalar("INSERT INTO sessions (user_id, token_hash, user_agent, ip, expires_at) VALUES ($1,$2,$3,$4::inet, now() + make_interval(days => $5)) RETURNING id")
        .bind(user_id).bind(hash_token(&token)).bind(ua).bind(&ip).bind(SESSION_DAYS as i32).fetch_one(&mut *tx).await?;
    crate::security::audit(
        &mut *tx,
        user_id,
        "auth.login",
        "sessions",
        sid,
        ip.as_deref(),
        json!({ "method": method }),
    )
    .await?;
    tx.commit().await?;
    Ok(token)
}

/// 以 (provider, uid) 找或建 user；回傳 (user_id, is_new)。
pub async fn upsert_identity(
    pool: &PgPool,
    provider: &str,
    uid: &str,
    email: Option<&str>,
    name: &str,
    avatar: Option<&str>,
) -> Result<(Uuid, bool), AppError> {
    let mut tx = pool.begin().await?;
    let result = upsert_identity_in_tx(&mut tx, provider, uid, email, name).await?;
    tx.commit().await?;
    crate::uploads::import_avatar(pool, result.0, provider, avatar).await;
    Ok(result)
}

pub(crate) async fn upsert_identity_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    provider: &str,
    uid: &str,
    email: Option<&str>,
    name: &str,
) -> Result<(Uuid, bool), AppError> {
    crate::identities::lock_identity(tx, provider, uid, email).await?;
    let mut found: Option<Uuid> = sqlx::query_scalar(
        "SELECT u.id FROM auth_identities i JOIN users u ON u.id=i.user_id
         WHERE i.provider=$1::auth_provider AND i.provider_uid=$2 AND u.deleted_at IS NULL FOR UPDATE OF u")
        .bind(provider).bind(uid).fetch_optional(&mut **tx).await?;
    // 等待 user 鎖時 identity 可能已被解除；用新 statement 重新核對 READ COMMITTED 的快照。
    if let Some(id) = found {
        found = sqlx::query_scalar("SELECT user_id FROM auth_identities WHERE provider=$1::auth_provider AND provider_uid=$2 AND user_id=$3")
            .bind(provider).bind(uid).bind(id).fetch_optional(&mut **tx).await?;
    }
    let mut existing: Vec<Uuid> = if found.is_none() {
        match email {
            Some(e) => sqlx::query_scalar("SELECT u.id FROM users u WHERE u.deleted_at IS NULL AND
                (u.email=$1 OR EXISTS(SELECT 1 FROM auth_identities i WHERE i.user_id=u.id AND i.email=$1)) FOR UPDATE OF u")
                .bind(e).fetch_all(&mut **tx).await?,
            None => vec![],
        }
    } else {
        vec![]
    };
    if !existing.is_empty() {
        // 供應商的不同信箱可能在等待 user 鎖期間被解除；取得鎖後再次檢查實際所有權。
        existing = sqlx::query_scalar("SELECT u.id FROM users u WHERE u.id=ANY($1) AND u.deleted_at IS NULL AND
            (u.email=$2 OR EXISTS(SELECT 1 FROM auth_identities i WHERE i.user_id=u.id AND i.email=$2))")
            .bind(&existing).bind(email).fetch_all(&mut **tx).await?;
    }
    if existing.len() > 1 {
        return Err(crate::identities::in_use());
    }
    let (id, is_new) = match found.or(existing.first().copied()) {
        Some(id) => (id, false),
        None => {
            let dn: String = name.chars().take(50).collect();
            let dn = if dn.trim().is_empty() {
                "user".to_string()
            } else {
                dn
            };
            (
                sqlx::query_scalar(
                    "INSERT INTO users (display_name,email) VALUES ($1,$2) RETURNING id",
                )
                .bind(dn)
                .bind(email)
                .fetch_one(&mut **tx)
                .await?,
                true,
            )
        }
    };
    let blocked: bool = sqlx::query_scalar("SELECT blocked_at IS NOT NULL FROM users WHERE id=$1")
        .bind(id)
        .fetch_one(&mut **tx)
        .await?;
    if blocked {
        return Err(AppError::problem(
            403,
            "ACCOUNT_BLOCKED",
            "帳號已停權，請聯絡客服",
        ));
    }
    let contact: Option<String> = sqlx::query_scalar("SELECT email FROM users WHERE id=$1")
        .bind(id)
        .fetch_one(&mut **tx)
        .await?;
    if provider == "email" {
        if contact.as_deref().is_some_and(|c| Some(c) != email) {
            return Err(crate::identities::email_mismatch());
        }
        sqlx::query("UPDATE users SET email=coalesce(email,$2) WHERE id=$1")
            .bind(id)
            .bind(email)
            .execute(&mut **tx)
            .await?;
    }
    if found.is_none() {
        sqlx::query("INSERT INTO auth_identities (user_id,provider,provider_uid,email) VALUES ($1,$2::auth_provider,$3,$4)")
            .bind(id).bind(provider).bind(uid).bind(email).execute(&mut **tx).await?;
        if !is_new {
            crate::identities::notify(
                tx,
                id,
                "auth.identity_linked",
                provider,
                contact.as_deref().or(email),
            )
            .await?;
        }
    }
    Ok((id, is_new))
}

// ---------- Email OTP ----------
#[derive(Deserialize)]
pub(crate) struct EmailReq {
    email: String,
    turnstile_token: Option<String>,
}

pub(crate) fn norm_email(e: &str) -> Result<String, AppError> {
    let e = e.trim().to_lowercase();
    let ok = e.len() <= 254
        && e.split_once('@').is_some_and(|(l, d)| {
            !l.is_empty() && d.contains('.') && !d.starts_with('.') && !d.ends_with('.')
        })
        && !e.chars().any(|c| c.is_whitespace() || c.is_control());
    if ok {
        Ok(e)
    } else {
        Err(AppError::invalid("/email", "FORMAT", "Email 格式不正確"))
    }
}

pub(crate) fn rate_limited(secs: i64) -> AppError {
    AppError::Problem {
        status: 429,
        code: "RATE_LIMITED",
        detail: format!("請求過於頻繁，請 {secs} 秒後再試。"),
        errors: None,
        retry_after: Some(secs.max(1) as u64),
    }
}

async fn send_mail(to: String, code: String, subject: String, lang: crate::notify::Lang) {
    use crate::notify::Lang::*;
    let body = match lang {
        Zh => format!("您的驗證碼是 {code}，10 分鐘內有效。若非本人操作請忽略此信。"),
        En => format!("Your verification code is {code}. It is valid for 10 minutes. If this wasn't you, please ignore this email."),
        Vi => format!("Mã xác minh của bạn là {code}, có hiệu lực trong 10 phút. Nếu không phải bạn yêu cầu, vui lòng bỏ qua email này."),
        Id => format!("Kode verifikasimu adalah {code}, berlaku selama 10 menit. Kalau bukan kamu yang meminta, abaikan saja email ini."),
    };
    // 驗證碼不進 DB 佇列（不落明文）；在 10 分鐘效期內就地重試，涵蓋 SMTP / API 短暫失敗。
    for (i, wait) in [0u64, 5, 30].into_iter().enumerate() {
        tokio::time::sleep(std::time::Duration::from_secs(wait)).await;
        match crate::notify::send_mail(&crate::config::get(), &to, &subject, &body).await {
            Ok(()) => return,
            Err(e) => tracing::error!(error = %e, attempt = i + 1, "send otp mail failed"),
        }
    }
}

// 硬上限以 (email, purpose) 計（每小時 20 封），避免匿名者用 login 用完額度而擋住本人的 reset / register / link。
// 匿名端點另有「軟上限 + Turnstile」與「每請求者 60 秒間隔」（otp_gate），每 IP 限流在各 handler。
/// 限流 + 寫入挑戰 + 寄信；回傳挑戰 id（驗證時綁定用）。
pub(crate) async fn issue_otp(
    pool: &PgPool,
    email: &str,
    purpose: &str,
    pw_hash: Option<&str>,
    name: Option<&str>,
) -> Result<Uuid, AppError> {
    issue_otp_scoped(pool, email, purpose, pw_hash, name, None, None, true).await
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn issue_otp_scoped(
    pool: &PgPool,
    email: &str,
    purpose: &str,
    pw_hash: Option<&str>,
    name: Option<&str>,
    owner: Option<Uuid>,
    session_hash: Option<&[u8]>,
    bound: bool,
) -> Result<Uuid, AppError> {
    let mut tx = pool.begin().await?;
    let (id, code) = issue_otp_in_tx(
        &mut tx,
        email,
        purpose,
        pw_hash,
        name,
        owner,
        session_hash,
        bound,
    )
    .await?;
    tx.commit().await?;
    send_otp(email, code, purpose);
    Ok(id)
}

/// 共用交易：回傳 (挑戰 id, 驗證碼)，呼叫者提交後才可寄信。鎖順序為 OTP → user。
#[allow(clippy::too_many_arguments)]
pub(crate) async fn issue_otp_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    email: &str,
    purpose: &str,
    pw_hash: Option<&str>,
    name: Option<&str>,
    owner: Option<Uuid>,
    session_hash: Option<&[u8]>,
    bound: bool,
) -> Result<(Uuid, String), AppError> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!("otp:{email}"))
        .execute(&mut **tx)
        .await?;
    if let Some(id) = owner {
        let live_user: Option<Uuid> = sqlx::query_scalar("SELECT id FROM users WHERE id=$1 AND deleted_at IS NULL AND blocked_at IS NULL FOR SHARE")
            .bind(id).fetch_optional(&mut **tx).await?;
        if live_user.is_none() {
            return Err(AppError::Unauthorized);
        }
        let live_session: Option<Uuid> = sqlx::query_scalar("SELECT id FROM sessions WHERE user_id=$1 AND token_hash=$2 AND revoked_at IS NULL AND expires_at>now() FOR SHARE")
            .bind(id).bind(session_hash).fetch_optional(&mut **tx).await?;
        if live_session.is_none() {
            return Err(AppError::Unauthorized);
        }
    }
    let cnt: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM otp_challenges WHERE email = $1 AND purpose = $2 AND created_at > now() - interval '1 hour'").bind(email).bind(purpose).fetch_one(&mut **tx).await?;
    if cnt >= 20 {
        return Err(rate_limited(600));
    }
    let code = format!("{:06}", rand::thread_rng().gen_range(0..1_000_000u32));
    let id: Uuid = sqlx::query_scalar("INSERT INTO otp_challenges (email, code_hash, expires_at, purpose, pending_password_hash, pending_display_name, link_user_id, link_session_hash, bound) VALUES ($1,$2, now() + make_interval(secs => $3),$4,$5,$6,$7,$8,$9) RETURNING id")
        .bind(email).bind(code_hash(&code, &pepper())).bind(OTP_TTL_SECS as f64).bind(purpose).bind(pw_hash).bind(name).bind(owner).bind(session_hash).bind(bound).fetch_one(&mut **tx).await?;
    Ok((id, code))
}

pub(crate) fn send_otp(email: &str, code: String, purpose: &str) {
    use crate::notify::Lang::*;
    let lang = crate::notify::request_lang();
    let subject = match (purpose, lang) {
        ("register", Zh) => "Wishly 註冊驗證碼",
        ("reset", Zh) => "Wishly 重設密碼驗證碼",
        ("link", Zh) => "Wishly 綁定 Email 驗證碼",
        (_, Zh) => "Wishly 登入驗證碼",
        ("register", En) => "Wishly sign-up verification code",
        ("reset", En) => "Wishly password reset verification code",
        ("link", En) => "Wishly email linking verification code",
        (_, En) => "Wishly sign-in verification code",
        ("register", Vi) => "Mã xác minh đăng ký Wishly",
        ("reset", Vi) => "Mã xác minh đặt lại mật khẩu Wishly",
        ("link", Vi) => "Mã xác minh liên kết Email Wishly",
        (_, Vi) => "Mã xác minh đăng nhập Wishly",
        ("register", Id) => "Kode verifikasi pendaftaran Wishly",
        ("reset", Id) => "Kode verifikasi atur ulang kata sandi Wishly",
        ("link", Id) => "Kode verifikasi penautan email Wishly",
        (_, Id) => "Kode verifikasi masuk Wishly",
    };
    tokio::spawn(send_mail(
        email.to_string(),
        code,
        subject.to_string(),
        lang,
    )); // 寄信失敗不影響回應（避免帳號列舉與外部依賴）
}

/// 匿名 OTP 請求的閘門（不論帳號是否存在都計數，不洩漏帳號）：
/// 每 (用途, email) 每小時超過 3 次要 Turnstile（軟上限，擋不住本人）；每 (IP, email) 60 秒間隔。
pub(crate) async fn otp_gate(
    pool: &PgPool,
    purpose: &str,
    email: &str,
    ip: &str,
    turnstile_token: Option<&str>,
) -> Result<(), AppError> {
    if !crate::ratelimit::allow(pool, &format!("otp_soft:{purpose}:{email}"), 3, 3600).await? {
        require_captcha(turnstile_token, ip).await?;
    }
    crate::ratelimit::check(pool, &format!("otp_gap:{purpose}:{ip}:{email}"), 1, 60).await
}

/// Turnstile 驗證；缺 / 錯 token 回 403 CAPTCHA_REQUIRED（前端據此顯示人機驗證）。
pub(crate) async fn require_captcha(
    turnstile_token: Option<&str>,
    ip: &str,
) -> Result<(), AppError> {
    match crate::reports::verify_turnstile(&crate::config::get(), turnstile_token, ip).await {
        Err(AppError::Problem { status: 403, .. }) => Err(AppError::problem(
            403,
            "CAPTCHA_REQUIRED",
            "請先完成人機驗證後再試。",
        )),
        r => r,
    }
}

/// 202 回應本體；challenge_id 供驗證時綁定。
pub(crate) fn otp_accepted(challenge_id: Uuid) -> (StatusCode, Json<Value>) {
    (
        StatusCode::ACCEPTED,
        Json(
            json!({ "expires_in": OTP_TTL_SECS, "resend_after": 60, "challenge_id": challenge_id }),
        ),
    )
}

async fn otp_request(
    State(st): State<AppState>,
    Peer(ip): Peer,
    Json(r): Json<EmailReq>,
) -> Result<(StatusCode, Json<Value>), AppError> {
    crate::ratelimit::check(&st.pool, &format!("otp_req:{ip}"), 20, 3600).await?;
    let email = norm_email(&r.email)?;
    otp_gate(&st.pool, "login", &email, &ip, r.turnstile_token.as_deref()).await?;
    let id = issue_otp(&st.pool, &email, "login", None, None).await?;
    Ok(otp_accepted(id))
}

#[derive(Deserialize)]
pub(crate) struct VerifyReq {
    pub email: String,
    pub code: String,
    pub challenge_id: Option<Uuid>,
}

pub(crate) fn otp_invalid() -> AppError {
    AppError::problem(400, "OTP_INVALID", "驗證碼錯誤或已過期，請重新取得。")
}

/// 驗證並消耗指定用途的 OTP；回傳 (pending_password_hash, pending_display_name)。
pub(crate) async fn consume_otp(
    pool: &PgPool,
    email: &str,
    code: &str,
    purpose: &str,
    challenge: Option<Uuid>,
) -> Result<(Option<String>, Option<String>), AppError> {
    consume_otp_scoped(pool, email, code, purpose, None, None, challenge).await
}

pub(crate) async fn consume_otp_scoped(
    pool: &PgPool,
    email: &str,
    code: &str,
    purpose: &str,
    owner: Option<Uuid>,
    session_hash: Option<&[u8]>,
    challenge: Option<Uuid>,
) -> Result<(Option<String>, Option<String>), AppError> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!("otp:{email}"))
        .execute(&mut *tx)
        .await?;
    if code.len() != 6 || !code.bytes().all(|b| b.is_ascii_digit()) {
        return Err(AppError::invalid("/code", "FORMAT", "驗證碼為 6 位數字"));
    }
    // 帶 challenge_id：只驗那一張、只套單張嘗試上限，他人亂猜 email 級額度也擋不了本人。
    // 沒帶（後台代寄的重設信、登入中綁定 Email）：只能驗未綁定的挑戰，每 (email, 用途) 每小時最多 10 次嘗試；不跨用途。
    // 匿名端點發出的挑戰一律綁定 id，他人不帶 id 亂猜碰不到本人那張。
    if challenge.is_none() {
        let tries: i64 = sqlx::query_scalar("SELECT coalesce(sum(attempts),0)::bigint FROM otp_challenges WHERE email=$1 AND purpose=$2 AND created_at > now() - interval '1 hour'")
            .bind(email).bind(purpose).fetch_one(&mut *tx).await?;
        if tries >= 10 {
            return Err(rate_limited(600));
        }
    }
    type OtpChallengeRow = (Uuid, Vec<u8>, i16, Option<String>, Option<String>);
    let ch: Option<OtpChallengeRow> = sqlx::query_as(
        "SELECT id, code_hash, attempts, pending_password_hash, pending_display_name FROM otp_challenges
         WHERE email=$1 AND purpose=$2 AND link_user_id IS NOT DISTINCT FROM $3 AND link_session_hash IS NOT DISTINCT FROM $4 AND consumed_at IS NULL AND expires_at > now()
         AND (id = $5 OR ($5::uuid IS NULL AND NOT bound))
         ORDER BY created_at DESC LIMIT 1").bind(email).bind(purpose).bind(owner).bind(session_hash).bind(challenge).fetch_optional(&mut *tx).await?;
    let (cid, hash, attempts, pw, name) = ch.ok_or_else(otp_invalid)?;
    if attempts >= OTP_MAX_ATTEMPTS {
        return Err(otp_invalid());
    }
    if !crate::config::ct_eq(&hash, &code_hash(code, &pepper())) {
        sqlx::query("UPDATE otp_challenges SET attempts = attempts + 1 WHERE id=$1")
            .bind(cid)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        return Err(otp_invalid());
    }
    // 一次性：只有第一個成功 UPDATE 的請求能通過
    let used = sqlx::query(
        "UPDATE otp_challenges SET consumed_at = now() WHERE id=$1 AND consumed_at IS NULL",
    )
    .bind(cid)
    .execute(&mut *tx)
    .await?;
    if used.rows_affected() != 1 {
        return Err(otp_invalid());
    }
    tx.commit().await?;
    Ok((pw, name))
}

/// 只有 Origin 屬於 NATIVE_ORIGINS（Tauri webview）且帶 X-Native-Client 才把權杖放進回應：
/// 網頁版 origin（APP_URL）即使送了 header 也拿不到，權杖仍只在 HttpOnly cookie。
/// 注意 Origin 可被非瀏覽器客戶端偽造，但那類客戶端本來就要有帳密，這道門擋的是「網頁內腳本」。
pub(crate) fn native_client(h: &HeaderMap) -> bool {
    let origin = h
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    h.contains_key("x-native-client")
        && crate::config::get()
            .native_origins
            .iter()
            .any(|o| o == origin)
}

/// 簽發 session 並回 {user, is_new_user} + Set-Cookie（OTP / 註冊 / 密碼登入共用）。
pub(crate) async fn session_response(
    pool: &PgPool,
    headers: &HeaderMap,
    ip: &str,
    method: &str,
    uid: Uuid,
    is_new: bool,
) -> Result<Response, AppError> {
    let token = new_session(
        pool,
        uid,
        headers
            .get(header::USER_AGENT)
            .and_then(|v| v.to_str().ok()),
        ip,
        method,
    )
    .await?;
    let u = user_json(pool, uid).await?;
    let mut body = json!({ "user": { "id": u["id"], "display_name": u["display_name"], "email": u["email"], "avatar_url": u["avatar_url"] }, "is_new_user": is_new });
    // 原生客戶端（Tauri）的 webview 不會帶跨站 cookie：帶 X-Native-Client 時把權杖放進回應，之後以 Bearer 使用。
    if native_client(headers) {
        body["session_token"] = json!(token);
    }
    Ok(([(header::SET_COOKIE, session_cookie(&token))], Json(body)).into_response())
}

async fn otp_verify(
    State(st): State<AppState>,
    Peer(ip): Peer,
    headers: HeaderMap,
    Json(r): Json<VerifyReq>,
) -> Result<Response, AppError> {
    crate::ratelimit::check(&st.pool, &format!("otp_ver:{ip}"), 30, 3600).await?;
    let email = norm_email(&r.email)?;
    consume_otp(&st.pool, &email, &r.code, "login", r.challenge_id).await?;
    let name = email.split('@').next().unwrap_or("user");
    let (uid, is_new) =
        upsert_identity(&st.pool, "email", &email, Some(&email), name, None).await?;
    session_response(&st.pool, &headers, &ip, "otp", uid, is_new).await
}

// ---------- logout / me ----------
async fn logout(
    u: CurrentUser,
    State(st): State<AppState>,
    Peer(ip): Peer,
) -> Result<Response, AppError> {
    let mut tx = st.pool.begin().await?;
    sqlx::query("UPDATE sessions SET revoked_at = now() WHERE id=$1 AND revoked_at IS NULL")
        .bind(u.session_id)
        .execute(&mut *tx)
        .await?;
    crate::security::audit(
        &mut *tx,
        u.id,
        "auth.logout",
        "sessions",
        u.session_id,
        crate::security::inet(&ip).as_deref(),
        json!({}),
    )
    .await?;
    tx.commit().await?;
    Ok((
        StatusCode::NO_CONTENT,
        [(
            header::SET_COOKIE,
            format!(
                "ws_session=; Max-Age=0; Path=/; HttpOnly; SameSite=Lax{}",
                secure()
            ),
        )],
    )
        .into_response())
}

pub(crate) async fn user_json(pool: &PgPool, id: Uuid) -> Result<Value, AppError> {
    let (dn, email, avatar, staff, prefs, handle, dcv): (String, Option<String>, Option<String>, bool, Value, Option<String>, String) =
        sqlx::query_as("SELECT display_name, email, avatar_key, is_staff, notification_prefs, handle, default_claim_visibility::text FROM users WHERE id=$1 AND deleted_at IS NULL")
            .bind(id).fetch_optional(pool).await?.ok_or(AppError::Unauthorized)?;
    let (bio, birthday, bv, locale): (Option<String>, Option<String>, String, String) =
        sqlx::query_as(
            "SELECT bio, birthday, birthday_visibility::text, locale FROM users WHERE id=$1",
        )
        .bind(id)
        .fetch_one(pool)
        .await?;
    let (mfa_enabled, has_password): (bool, bool) = sqlx::query_as(
        "SELECT totp_enabled_at IS NOT NULL, EXISTS(SELECT 1 FROM auth_identities i WHERE i.user_id=$1 AND i.provider='email' AND i.password_hash IS NOT NULL)
         FROM users WHERE id=$1")
        .bind(id).fetch_one(pool).await?;
    let idents: Vec<String> = sqlx::query_scalar(
        "SELECT provider::text FROM auth_identities WHERE user_id=$1 ORDER BY created_at",
    )
    .bind(id)
    .fetch_all(pool)
    .await?;
    let avatar_url = crate::uploads::avatar_url(avatar.as_deref());
    let orgs: Vec<Value> = sqlx::query_as::<_, (Uuid, String, String, String)>(
        "SELECT o.id, o.display_name, m.role::text, o.verification_status FROM org_members m JOIN organizations o ON o.id = m.org_id
         WHERE m.user_id = $1 AND m.disabled_at IS NULL ORDER BY o.created_at")
        .bind(id).fetch_all(pool).await?
        .into_iter().map(|(i, n, r, v)| json!({"id": i, "name": n, "role": r, "verification_status": v})).collect();
    Ok(
        json!({ "id": id, "display_name": dn, "email": email, "avatar_url": avatar_url, "is_staff": staff,
               "notification_prefs": prefs, "handle": handle, "default_claim_visibility": dcv, "bio": bio, "birthday": birthday, "birthday_visibility": bv, "locale": locale, "mfa_enabled": mfa_enabled, "has_password": has_password, "identities": idents.iter().map(|p| json!({"provider": p})).collect::<Vec<_>>(),
               "orgs": orgs }),
    )
}

async fn me(u: CurrentUser, State(st): State<AppState>) -> Result<Json<Value>, AppError> {
    let mut v = user_json(&st.pool, u.id).await?;
    v["mfa_verified"] = json!(u.mfa); // 只有 GET /me 知道目前 session
    Ok(Json(v))
}

fn some_opt<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Option<String>>, D::Error> {
    Option::<String>::deserialize(d).map(Some)
}

#[derive(Deserialize)]
struct PatchMe {
    display_name: Option<String>,
    notification_prefs: Option<Value>,
    /// 缺欄位 = 不改；null = 清除
    #[serde(default, deserialize_with = "some_opt")]
    handle: Option<Option<String>>,
    default_claim_visibility: Option<String>,
    #[serde(default, deserialize_with = "some_opt")]
    bio: Option<Option<String>>,
    /// 'MM-DD'；null = 清除
    #[serde(default, deserialize_with = "some_opt")]
    birthday: Option<Option<String>>,
    birthday_visibility: Option<String>,
    /// 介面語言（信件語言）：zh-TW / en / vi
    locale: Option<String>,
}

fn share_level(v: &str, ptr: &'static str) -> Result<(), AppError> {
    if ["public", "friends", "private"].contains(&v) {
        Ok(())
    } else {
        Err(AppError::invalid(
            ptr,
            "ENUM",
            "只能是 public / friends / private",
        ))
    }
}

async fn patch_me(
    u: CurrentUser,
    State(st): State<AppState>,
    Json(p): Json<PatchMe>,
) -> Result<Json<Value>, AppError> {
    if let Some(n) = &p.display_name {
        if !(1..=50).contains(&n.trim().chars().count()) {
            return Err(AppError::invalid(
                "/display_name",
                "RANGE",
                "暱稱長度須為 1–50 字",
            ));
        }
        sqlx::query("UPDATE users SET display_name=$2 WHERE id=$1")
            .bind(u.id)
            .bind(n.trim())
            .execute(&st.pool)
            .await?;
    }
    if let Some(prefs) = p.notification_prefs {
        let ok = prefs.as_object().is_some_and(|o| {
            !o.is_empty() && o.iter().all(|(k, v)| k == "email_claims" && v.is_boolean())
        });
        if !ok {
            return Err(AppError::invalid(
                "/notification_prefs",
                "FORMAT",
                "僅支援布林值的 email_claims",
            ));
        }
        sqlx::query("UPDATE users SET notification_prefs = notification_prefs || $2 WHERE id=$1")
            .bind(u.id)
            .bind(prefs)
            .execute(&st.pool)
            .await?;
    }
    if let Some(h) = p.handle {
        let h = h.map(|h| h.trim().to_lowercase());
        if h.as_deref().is_some_and(|h| {
            !(3..=30).contains(&h.len())
                || !h
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        }) {
            return Err(AppError::invalid(
                "/handle",
                "FORMAT",
                "帳號代號需為 3–30 碼小寫英數或底線",
            ));
        }
        match sqlx::query("UPDATE users SET handle=$2 WHERE id=$1")
            .bind(u.id)
            .bind(h)
            .execute(&st.pool)
            .await
        {
            Err(sqlx::Error::Database(e)) if e.constraint() == Some("users_handle_key") => {
                return Err(AppError::problem(409, "HANDLE_TAKEN", "此帳號代號已被使用"))
            }
            r => {
                r?;
            }
        }
    }
    if let Some(v) = &p.default_claim_visibility {
        share_level(v, "/default_claim_visibility")?;
        sqlx::query("UPDATE users SET default_claim_visibility=$2::text::share_level WHERE id=$1")
            .bind(u.id)
            .bind(v)
            .execute(&st.pool)
            .await?;
    }
    if let Some(b) = p.bio {
        // 空白視同清除
        let b = b.map(|b| b.trim().to_string()).filter(|b| !b.is_empty());
        if b.as_ref().is_some_and(|b| b.chars().count() > 160) {
            return Err(AppError::invalid("/bio", "RANGE", "自我介紹最多 160 字"));
        }
        sqlx::query("UPDATE users SET bio=$2 WHERE id=$1")
            .bind(u.id)
            .bind(b)
            .execute(&st.pool)
            .await?;
    }
    if let Some(b) = p.birthday {
        // 用閏年 2000 驗證，允許 02-29；格式須剛好 MM-DD
        if b.as_deref().is_some_and(|b| {
            b.len() != 5
                || chrono::NaiveDate::parse_from_str(&format!("2000-{b}"), "%Y-%m-%d").is_err()
        }) {
            return Err(AppError::invalid(
                "/birthday",
                "FORMAT",
                "生日格式須為 MM-DD",
            ));
        }
        sqlx::query("UPDATE users SET birthday=$2 WHERE id=$1")
            .bind(u.id)
            .bind(b)
            .execute(&st.pool)
            .await?;
    }
    if let Some(v) = &p.birthday_visibility {
        share_level(v, "/birthday_visibility")?;
        sqlx::query("UPDATE users SET birthday_visibility=$2::text::share_level WHERE id=$1")
            .bind(u.id)
            .bind(v)
            .execute(&st.pool)
            .await?;
    }
    if let Some(v) = &p.locale {
        if !["zh-TW", "en", "vi", "id"].contains(&v.as_str()) {
            return Err(AppError::invalid(
                "/locale",
                "ENUM",
                "只能是 zh-TW / en / vi / id",
            ));
        }
        sqlx::query("UPDATE users SET locale=$2 WHERE id=$1")
            .bind(u.id)
            .bind(v)
            .execute(&st.pool)
            .await?;
    }
    Ok(Json(user_json(&st.pool, u.id).await?))
}

// ---------- LINE Login ----------
/// HTTP 薄層設定：全部由環境變數驅動，URL 可指向 mock。
pub struct LineCfg {
    pub auth_url: String,
    pub token_url: String,
    pub profile_url: String,
    pub client_id: String,
    pub secret: String,
    pub redirect_uri: String,
}
impl LineCfg {
    pub fn from_env() -> Option<LineCfg> {
        Some(LineCfg {
            client_id: crate::auth_ext::nonempty("LINE_CHANNEL_ID")?,
            secret: crate::auth_ext::nonempty("LINE_CHANNEL_SECRET")?,
            redirect_uri: env(
                "LINE_REDIRECT_URI",
                "http://localhost:8080/api/v1/auth/oauth/line/callback",
            ),
            auth_url: env(
                "LINE_AUTH_URL",
                "https://access.line.me/oauth2/v2.1/authorize",
            ),
            token_url: env("LINE_TOKEN_URL", "https://api.line.me/oauth2/v2.1/token"),
            profile_url: env("LINE_PROFILE_URL", "https://api.line.me/v2/profile"),
        })
    }
}
pub struct LineProfile {
    pub user_id: String,
    pub display_name: String,
    pub picture_url: Option<String>,
}

pub async fn line_fetch_profile(
    cfg: &LineCfg,
    code: &str,
    verifier: &str,
) -> Result<LineProfile, AppError> {
    let bad = |e: String| {
        tracing::error!(error = %e, "line oauth");
        AppError::problem(502, "OAUTH_PROVIDER_ERROR", "LINE 登入暫時無法使用")
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
    let at = tok["access_token"]
        .as_str()
        .ok_or_else(|| bad("no access_token".into()))?;
    let p: Value = c
        .get(&cfg.profile_url)
        .bearer_auth(at)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| bad(e.to_string()))?
        .json()
        .await
        .map_err(|e| bad(e.to_string()))?;
    Ok(LineProfile {
        user_id: p["userId"]
            .as_str()
            .ok_or_else(|| bad("no userId".into()))?
            .to_string(),
        display_name: p["displayName"]
            .as_str()
            .unwrap_or("LINE 使用者")
            .to_string(),
        picture_url: p["pictureUrl"].as_str().map(String::from),
    })
}

pub(crate) fn sign(payload: &str) -> String {
    let mut m = <Hmac<Sha256> as Mac>::new_from_slice(crate::config::get().oauth_secret.as_bytes())
        .unwrap();
    m.update(payload.as_bytes());
    hex::encode(m.finalize().into_bytes())
}
pub(crate) fn safe_redirect(r: &str) -> bool {
    r.starts_with('/')
        && !r.starts_with("//")
        && !r.contains('\\')
        && !r.contains(|c: char| c.is_control())
}
pub(crate) fn rand_b64(n: usize) -> String {
    let mut b = vec![0u8; n];
    rand::thread_rng().fill_bytes(&mut b);
    B64.encode(b)
}

#[derive(Deserialize)]
struct StartQ {
    redirect: Option<String>,
    #[serde(default)]
    link: bool,
    /// 原生殼：完成後以 deep link 帶一次性碼回 App（須搭配 app_challenge；不支援 link）
    #[serde(default)]
    native: bool,
    app_challenge: Option<String>,
    /// 原生殼綁定：由 POST /auth/oauth/{provider}/link-ticket 取得的一次性 ticket
    ticket: Option<String>,
}

#[derive(Deserialize, Serialize)]
struct OAuthState {
    state: String,
    verifier: String,
    redirect: String,
    provider: String,
    issued_at: i64,
    link_user: Option<Uuid>,
    session_hash: Option<String>,
    #[serde(default)]
    app_challenge: Option<String>,
}

/// 原生殼 deep link 基底（App 註冊的 scheme）；結果以 ?code= 或 ?error= 附帶。
fn native_redirect() -> String {
    env("NATIVE_REDIRECT", "wishly://auth")
}

/// PKCE S256 的 challenge 為 43 字元 base64url。
fn valid_challenge(c: &str) -> bool {
    c.len() == 43
        && c.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

async fn oauth_start(
    State(st): State<AppState>,
    Path(provider): Path<String>,
    Query(q): Query<StartQ>,
    mut parts: axum::http::request::Parts,
) -> Result<Response, AppError> {
    if provider != "line" && provider != "google" {
        return Err(AppError::invalid(
            "/provider",
            "ENUM",
            "provider 須為 line 或 google",
        ));
    }
    let app_challenge = if q.native {
        match q.app_challenge.as_deref() {
            Some(c) if valid_challenge(c) => Some(c.to_string()),
            _ => {
                return Err(AppError::invalid(
                    "/app_challenge",
                    "FORMAT",
                    "native 需要 43 字元的 app_challenge",
                ))
            }
        }
    } else {
        None
    };
    let (link_user, session_hash) = if let (true, Some(challenge)) = (q.link, &app_challenge) {
        // 原生殼：系統瀏覽器沒有 session，改以 App 預先換好的 ticket 帶出身分（單次、綁 app_challenge）
        let ticket = q
            .ticket
            .as_deref()
            .ok_or_else(|| AppError::invalid("/ticket", "REQUIRED", "原生綁定需要 ticket"))?;
        let row: Option<(Uuid, Option<Vec<u8>>)> = sqlx::query_as(
            "DELETE FROM native_auth_codes WHERE code_hash=$1 AND purpose='link' AND challenge=$2 AND expires_at > now() RETURNING user_id, session_hash",
        )
        .bind(hash_token(ticket))
        .bind(challenge)
        .fetch_optional(&st.pool)
        .await?;
        let (uid, sh) = row.ok_or(AppError::Unauthorized)?;
        (Some(uid), sh.map(hex::encode))
    } else if q.link {
        let u = CurrentUser::from_request_parts(&mut parts, &st).await?;
        crate::ratelimit::check(&st.pool, &format!("identity-oauth:{}", u.id), 20, 3600).await?;
        (
            Some(u.id),
            crate::session::session_token(&parts.headers).map(|t| hex::encode(hash_token(&t))),
        )
    } else {
        (None, None)
    };
    let redirect = if q.link {
        "/settings".into()
    } else {
        q.redirect.unwrap_or_else(|| "/dashboard".into())
    };
    if !safe_redirect(&redirect) {
        return Err(AppError::invalid(
            "/redirect",
            "FORMAT",
            "redirect 必須是以 / 開頭的相對路徑",
        ));
    }
    // 未設憑證：瀏覽器是整頁導向進來的，回 302 到登入頁讓前端顯示提示，而不是一頁 JSON
    let unavailable = || {
        (
            StatusCode::FOUND,
            [(
                header::LOCATION,
                if app_challenge.is_some() {
                    format!("{}?error=oauth_unavailable", native_redirect())
                } else {
                    format!(
                        "{}/{}?error=oauth_unavailable",
                        app_url(),
                        if q.link { "settings" } else { "login" }
                    )
                },
            )],
        )
            .into_response()
    };
    let (state, verifier) = (rand_b64(16), rand_b64(32));
    let challenge = B64.encode(Sha256::digest(verifier.as_bytes()));
    let (auth_url, params): (String, Vec<(&str, String)>) = if provider == "line" {
        let Some(cfg) = LineCfg::from_env() else {
            return Ok(unavailable());
        };
        (
            cfg.auth_url,
            vec![
                ("client_id", cfg.client_id),
                ("redirect_uri", cfg.redirect_uri),
                ("scope", "profile openid".into()),
            ],
        )
    } else {
        let Some(cfg) = crate::auth_ext::GoogleCfg::from_env() else {
            return Ok(unavailable());
        };
        (
            cfg.auth_url,
            vec![
                ("client_id", cfg.client_id),
                ("redirect_uri", cfg.redirect_uri),
                ("scope", "openid email profile".into()),
            ],
        )
    };
    let mut all: Vec<(&str, String)> = vec![
        ("response_type", "code".into()),
        ("state", state.clone()),
        ("code_challenge", challenge),
        ("code_challenge_method", "S256".into()),
    ];
    all.extend(params);
    let url = reqwest::Url::parse_with_params(&auth_url, all)
        .map_err(|e| AppError::problem(502, "OAUTH_PROVIDER_ERROR", e.to_string()))?;
    let payload = B64.encode(
        serde_json::to_vec(&OAuthState {
            state,
            verifier,
            redirect,
            provider,
            issued_at: chrono::Utc::now().timestamp(),
            link_user,
            session_hash,
            app_challenge,
        })
        .unwrap(),
    );
    let ck = format!("ws_oauth_state={payload}.{}; Path=/api/v1/auth/oauth; Max-Age=600; HttpOnly; SameSite=Lax{}", sign(&payload), secure());
    Ok((
        StatusCode::FOUND,
        [
            (header::LOCATION, url.to_string()),
            (header::SET_COOKIE, ck),
        ],
    )
        .into_response())
}

#[derive(Deserialize)]
struct CbQ {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

/// 簽章保護供應商、期限、用途、帳號與發起 session，不依賴瀏覽器 cookie 的 Max-Age。
fn check_state(cookie_val: Option<&str>, state: Option<&str>) -> Option<OAuthState> {
    let (payload, sig) = cookie_val?.rsplit_once('.')?;
    if !crate::config::ct_eq(sign(payload).as_bytes(), sig.as_bytes()) {
        return None;
    }
    let s: OAuthState = serde_json::from_slice(&B64.decode(payload).ok()?).ok()?;
    let age = chrono::Utc::now().timestamp() - s.issued_at;
    if !(0..=600).contains(&age)
        || !crate::config::ct_eq(s.state.as_bytes(), state?.as_bytes())
        || !safe_redirect(&s.redirect)
    {
        return None;
    }
    Some(s)
}

fn oauth_error(error: AppError) -> &'static str {
    match error {
        AppError::Problem {
            code: "ACCOUNT_BLOCKED",
            ..
        } => "account_blocked",
        AppError::Problem {
            code: "IDENTITY_IN_USE",
            ..
        } => "identity_in_use",
        AppError::Problem {
            code: "PROVIDER_ALREADY_LINKED",
            ..
        } => "provider_already_linked",
        error => {
            tracing::warn!(?error, "oauth callback failed");
            "oauth_failed"
        }
    }
}

async fn oauth_callback(
    State(st): State<AppState>,
    Path(provider): Path<String>,
    Query(q): Query<CbQ>,
    Peer(ip): Peer,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let clear = format!(
        "ws_oauth_state=; Max-Age=0; Path=/api/v1/auth/oauth; HttpOnly; SameSite=Lax{}",
        secure()
    );
    let state = check_state(
        cookie(&headers, "ws_oauth_state").as_deref(),
        q.state.as_deref(),
    );
    let linking = state.as_ref().is_some_and(|s| s.link_user.is_some());
    let native = state.as_ref().is_some_and(|s| s.app_challenge.is_some());
    let fail = |error: &str| {
        let mut response = (
            StatusCode::FOUND,
            AppendHeaders([
                (
                    header::LOCATION,
                    if native {
                        format!("{}?error={error}", native_redirect())
                    } else {
                        format!(
                            "{}/{}?error={error}",
                            app_url(),
                            if linking && error != "account_blocked" {
                                "settings"
                            } else {
                                "login"
                            }
                        )
                    },
                ),
                (header::SET_COOKIE, clear.clone()),
            ]),
        )
            .into_response();
        if error == "account_blocked" {
            response.headers_mut().append(
                header::SET_COOKIE,
                format!(
                    "ws_session=; Max-Age=0; Path=/; HttpOnly; SameSite=Lax{}",
                    secure()
                )
                .parse()
                .unwrap(),
            );
        }
        response
    };
    let (Some(code), Some(s)) = (q.code.as_deref(), state) else {
        return Ok(fail("oauth_failed"));
    };
    if s.provider != provider
        || q.error.is_some()
        || !["line", "google"].contains(&provider.as_str())
    {
        return Ok(fail("oauth_failed"));
    }
    // 原生殼綁定：callback 在系統瀏覽器，沒有 session；身分與 session 已由 ticket 簽進 state
    let session_hash = if s.app_challenge.is_some() {
        s.session_hash.as_deref().and_then(|h| hex::decode(h).ok())
    } else {
        crate::session::session_token(&headers).map(|t| hash_token(&t))
    };
    if let Some(id) = s.link_user {
        if session_hash.as_ref().map(hex::encode) != s.session_hash {
            return Ok(fail("oauth_failed"));
        }
        let live: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sessions s JOIN users u ON u.id=s.user_id WHERE s.user_id=$1 AND s.token_hash=$2 AND s.revoked_at IS NULL AND s.expires_at > now() AND u.deleted_at IS NULL AND u.blocked_at IS NULL)")
            .bind(id).bind(&session_hash).fetch_one(&st.pool).await?;
        if !live {
            return Ok(fail("oauth_failed"));
        }
    }
    let identity = if provider == "line" {
        let Some(cfg) = LineCfg::from_env() else {
            return Ok(fail("oauth_unavailable"));
        };
        match line_fetch_profile(&cfg, code, &s.verifier).await {
            Ok(p) => (p.user_id, None, p.display_name, p.picture_url),
            Err(_) => return Ok(fail("oauth_failed")),
        }
    } else {
        match crate::auth_ext::google_identity(code, &s.verifier).await {
            Ok(Some(g)) => (g.sub, g.email, g.name, g.picture),
            _ => return Ok(fail("oauth_failed")),
        }
    };
    let (sub, email, name, picture) = identity;
    // 原生殼綁定：瀏覽器不一定是發 ticket 的那台裝置，這裡不寫 auth_identities，
    // 只存待確認結果，由 App 憑 verifier + 同 session 呼叫 link-confirm 才綁定。
    if let (Some(id), Some(challenge)) = (s.link_user, &s.app_challenge) {
        let code = rand_b64(32);
        sqlx::query("INSERT INTO native_auth_codes (code_hash, user_id, challenge, expires_at, purpose, session_hash, provider, provider_uid, email, picture) VALUES ($1,$2,$3, now() + interval '60 seconds','link_result',$4,$5,$6,$7,$8)")
            .bind(hash_token(&code)).bind(id).bind(challenge).bind(session_hash.as_deref())
            .bind(&provider).bind(&sub).bind(email.as_deref()).bind(picture.as_deref())
            .execute(&st.pool).await?;
        return Ok((
            StatusCode::FOUND,
            AppendHeaders([
                (
                    header::LOCATION,
                    format!("{}?link_code={code}", native_redirect()),
                ),
                (header::SET_COOKIE, clear),
            ]),
        )
            .into_response());
    }
    if let Some(id) = s.link_user {
        match crate::identities::link(
            &st.pool,
            id,
            session_hash.as_deref().unwrap_or_default(),
            &provider,
            &sub,
            email.as_deref(),
        )
        .await
        {
            Ok(()) => {
                crate::uploads::import_avatar(&st.pool, id, &provider, picture.as_deref()).await;
                return Ok((
                    StatusCode::FOUND,
                    AppendHeaders([
                        (
                            header::LOCATION,
                            format!("{}/settings?linked={provider}", app_url()),
                        ),
                        (header::SET_COOKIE, clear),
                    ]),
                )
                    .into_response());
            }
            Err(error) => return Ok(fail(oauth_error(error))),
        }
    }
    let uid = match upsert_identity(
        &st.pool,
        &provider,
        &sub,
        email.as_deref(),
        &name,
        picture.as_deref(),
    )
    .await
    {
        Ok((uid, _)) => uid,
        Err(error) => return Ok(fail(oauth_error(error))),
    };
    // 原生殼：不在這裡建 session（瀏覽器的 cookie 對 App 無用），改發一次性碼，由 App 憑 verifier 兌換。
    if let Some(challenge) = &s.app_challenge {
        let code = rand_b64(32);
        sqlx::query("INSERT INTO native_auth_codes (code_hash, user_id, challenge, expires_at) VALUES ($1,$2,$3, now() + interval '60 seconds')")
            .bind(hash_token(&code)).bind(uid).bind(challenge).execute(&st.pool).await?;
        return Ok((
            StatusCode::FOUND,
            AppendHeaders([
                (
                    header::LOCATION,
                    format!("{}?code={code}", native_redirect()),
                ),
                (header::SET_COOKIE, clear),
            ]),
        )
            .into_response());
    }
    let login = new_session(
        &st.pool,
        uid,
        headers
            .get(header::USER_AGENT)
            .and_then(|v| v.to_str().ok()),
        &ip,
        &provider,
    )
    .await;
    let token = match login {
        Ok(token) => token,
        Err(error) => return Ok(fail(oauth_error(error))),
    };
    Ok((
        StatusCode::FOUND,
        AppendHeaders([
            (header::LOCATION, format!("{}{}", app_url(), s.redirect)),
            (header::SET_COOKIE, session_cookie(&token)),
            (header::SET_COOKIE, clear),
        ]),
    )
        .into_response())
}

#[derive(Deserialize)]
struct TicketReq {
    app_challenge: String,
}

/// 原生殼綁定第三方帳號：以 Bearer 身分換一張 60 秒、單次、綁 app_challenge 與目前 session 的 ticket。
async fn link_ticket(
    State(st): State<AppState>,
    Path(provider): Path<String>,
    headers: HeaderMap,
    u: CurrentUser,
    Json(r): Json<TicketReq>,
) -> Result<Json<Value>, AppError> {
    if !native_client(&headers) {
        return Err(AppError::problem(403, "NATIVE_ONLY", "僅限原生客戶端"));
    }
    if provider != "line" && provider != "google" {
        return Err(AppError::invalid(
            "/provider",
            "ENUM",
            "provider 須為 line 或 google",
        ));
    }
    if !valid_challenge(&r.app_challenge) {
        return Err(AppError::invalid(
            "/app_challenge",
            "FORMAT",
            "需要 43 字元的 app_challenge",
        ));
    }
    crate::ratelimit::check(&st.pool, &format!("identity-oauth:{}", u.id), 20, 3600).await?;
    let session_hash = crate::session::session_token(&headers).map(|t| hash_token(&t));
    let ticket = rand_b64(32);
    sqlx::query("INSERT INTO native_auth_codes (code_hash, user_id, challenge, expires_at, purpose, session_hash) VALUES ($1,$2,$3, now() + interval '60 seconds','link',$4)")
        .bind(hash_token(&ticket)).bind(u.id).bind(&r.app_challenge).bind(session_hash).execute(&st.pool).await?;
    Ok(Json(json!({ "ticket": ticket })))
}

#[derive(Deserialize)]
struct ExchangeReq {
    code: String,
    verifier: String,
}

/// 原生殼以 deep link 帶回的一次性碼 + PKCE verifier 兌換 session 權杖。
/// 碼先刪除再驗證：不論成敗都只能用一次，攔截者只有一次機會且沒有 verifier。
async fn native_exchange(
    State(st): State<AppState>,
    Peer(ip): Peer,
    headers: HeaderMap,
    Json(r): Json<ExchangeReq>,
) -> Result<Response, AppError> {
    if !native_client(&headers) {
        return Err(AppError::problem(403, "NATIVE_ONLY", "僅限原生客戶端"));
    }
    let invalid =
        || AppError::problem(400, "NATIVE_CODE_INVALID", "登入碼無效或已過期，請重新登入");
    let (uid, challenge): (Uuid, String) = sqlx::query_as(
        "DELETE FROM native_auth_codes WHERE code_hash=$1 AND purpose='login' AND expires_at > now() RETURNING user_id, challenge",
    )
    .bind(hash_token(&r.code))
    .fetch_optional(&st.pool)
    .await?
    .ok_or_else(invalid)?;
    let expect = B64.encode(Sha256::digest(r.verifier.as_bytes()));
    if !crate::config::ct_eq(expect.as_bytes(), challenge.as_bytes()) {
        return Err(invalid());
    }
    session_response(&st.pool, &headers, &ip, "native", uid, false).await
}

/// 原生殼綁定的最後一步：link_code + PKCE verifier + 發起 ticket 的同一個 Bearer session，才寫入 auth_identities。
/// 碼先刪除再驗證（單次）；purpose 分開，link_result 碼無法當登入碼兌換，反之亦然。
async fn native_link_confirm(
    State(st): State<AppState>,
    headers: HeaderMap,
    u: CurrentUser,
    Json(r): Json<ExchangeReq>,
) -> Result<Json<Value>, AppError> {
    if !native_client(&headers) {
        return Err(AppError::problem(403, "NATIVE_ONLY", "僅限原生客戶端"));
    }
    let invalid =
        || AppError::problem(400, "NATIVE_CODE_INVALID", "綁定碼無效或已過期，請重新操作");
    #[allow(clippy::type_complexity)]
    let (uid, challenge, hash, provider, puid, email, picture): (
        Uuid,
        String,
        Option<Vec<u8>>,
        String,
        String,
        Option<String>,
        Option<String>,
    ) = sqlx::query_as(
        "DELETE FROM native_auth_codes WHERE code_hash=$1 AND purpose='link_result' AND expires_at > now() RETURNING user_id, challenge, session_hash, provider, provider_uid, email, picture",
    )
    .bind(hash_token(&r.code))
    .fetch_optional(&st.pool)
    .await?
    .ok_or_else(invalid)?;
    let expect = B64.encode(Sha256::digest(r.verifier.as_bytes()));
    let cur = crate::session::session_token(&headers).map(|t| hash_token(&t));
    if uid != u.id
        || !crate::config::ct_eq(expect.as_bytes(), challenge.as_bytes())
        || hash.is_none()
        || hash != cur
    {
        return Err(invalid());
    }
    crate::identities::link(
        &st.pool,
        uid,
        hash.as_deref().unwrap_or_default(),
        &provider,
        &puid,
        email.as_deref(),
    )
    .await?;
    crate::uploads::import_avatar(&st.pool, uid, &provider, picture.as_deref()).await;
    Ok(Json(json!({ "linked": provider })))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_token_only_for_native_origins() {
        let mk = |origin: Option<&str>, native: bool| {
            let mut h = HeaderMap::new();
            if let Some(o) = origin {
                h.insert(header::ORIGIN, o.parse().unwrap());
            }
            if native {
                h.insert("x-native-client", "1".parse().unwrap());
            }
            h
        };
        assert!(native_client(&mk(Some("tauri://localhost"), true)));
        assert!(
            !native_client(&mk(Some("tauri://localhost"), false)),
            "沒帶 X-Native-Client"
        );
        assert!(
            !native_client(&mk(Some(&crate::config::get().app_url), true)),
            "網頁版 origin 不發權杖"
        );
        assert!(!native_client(&mk(Some("https://evil.example"), true)));
        assert!(!native_client(&mk(None, true)), "沒有 Origin");
    }
    #[test]
    fn state_cookie_roundtrip_and_tamper() {
        let payload = B64.encode(
            serde_json::to_vec(&OAuthState {
                state: "st1".into(),
                verifier: "ver1".into(),
                redirect: "/dashboard".into(),
                provider: "google".into(),
                issued_at: chrono::Utc::now().timestamp(),
                link_user: None,
                session_hash: None,
                app_challenge: None,
            })
            .unwrap(),
        );
        let c = format!("{payload}.{}", sign(&payload));
        let state = check_state(Some(&c), Some("st1")).unwrap();
        assert_eq!(
            (state.verifier.as_str(), state.redirect.as_str()),
            ("ver1", "/dashboard")
        );
        assert!(check_state(Some(&c), Some("other")).is_none());
        assert!(check_state(Some(&format!("x{c}")), Some("st1")).is_none());
        assert!(check_state(None, Some("st1")).is_none());
        assert!(
            !safe_redirect("//evil.com") && !safe_redirect("https://x") && safe_redirect("/a/b")
        );
    }
}
