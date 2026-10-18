//! 帳號安全：營運人員 TOTP 兩步驟驗證、登入裝置（session）管理、登入狀態下改密碼、使用者安全事件稽核。
//! TOTP 只開放 is_staff 設定；啟用後 `admin::Staff` 守衛要求 session 已通過（sessions.mfa_at）。
use crate::{error::AppError, ratelimit::Peer, session::CurrentUser, AppState};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, get, post},
    Json, Router,
};
use hmac::{Hmac, Mac};
use rand::RngCore;
use serde::Deserialize;
use serde_json::{json, Value};
use sha1::Sha1;
use uuid::Uuid;

pub fn routes() -> Router<AppState> {
    // /auth/mfa/* 在唯讀模式也放行（lib.rs read_only），否則 staff 無法驗證後關閉唯讀
    Router::new()
        .route("/auth/mfa/totp/setup", post(totp_setup))
        .route("/auth/mfa/totp/enable", post(totp_enable))
        .route("/auth/mfa/verify", post(mfa_verify))
        .route("/me/sessions", get(list_sessions).delete(revoke_others))
        .route("/me/sessions/{id}", delete(revoke_session))
        .route("/me/password", post(change_password))
}

/// client_ip 可能是 "unknown"；只有合法 IP 才寫進 inet 欄位。
pub(crate) fn inet(ip: &str) -> Option<String> {
    ip.parse::<std::net::IpAddr>().ok().map(|i| i.to_string())
}

/// 使用者本人的安全事件（auth.login / auth.logout / auth.password_* / auth.mfa_* / auth.session*）
pub(crate) async fn audit<'e, E: sqlx::PgExecutor<'e>>(
    ex: E,
    uid: Uuid,
    action: &str,
    entity: &str,
    entity_id: Uuid,
    ip: Option<&str>,
    diff: Value,
) -> Result<(), AppError> {
    sqlx::query("INSERT INTO audit_logs (actor_type, actor_id, action, entity, entity_id, diff, ip) VALUES ('user',$1,$2,$3,$4,$5,$6::inet)")
        .bind(uid).bind(action).bind(entity).bind(entity_id).bind(diff).bind(ip).execute(ex).await?;
    Ok(())
}

// ---------- TOTP（RFC 6238：SHA-1、6 碼、30 秒） ----------
const STEP: u64 = 30;

fn hotp(secret: &[u8], counter: u64) -> u32 {
    let mut m = <Hmac<Sha1> as Mac>::new_from_slice(secret).expect("hmac 接受任意長度金鑰");
    m.update(&counter.to_be_bytes());
    let h = m.finalize().into_bytes();
    let o = (h[19] & 0x0f) as usize;
    (u32::from_be_bytes([h[o], h[o + 1], h[o + 2], h[o + 3]]) & 0x7fff_ffff) % 1_000_000
}

/// 符合的時間步；前後各容忍 1 步（時鐘誤差）
fn totp_match(secret: &[u8], code: &str, unix: u64) -> Option<i64> {
    if code.len() != 6 || !code.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let now = unix / STEP;
    (now.saturating_sub(1)..=now + 1)
        .find(|&s| {
            crate::config::ct_eq(
                format!("{:06}", hotp(secret, s)).as_bytes(),
                code.as_bytes(),
            )
        })
        .map(|s| s as i64)
}

/// RFC 4648 base32，不補 `=`（驗證器 App 的格式）
fn base32(b: &[u8]) -> String {
    const A: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let (mut out, mut buf, mut bits) = (String::new(), 0u32, 0);
    for &x in b {
        buf = (buf << 8) | x as u32;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(A[((buf >> bits) & 31) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(A[((buf << (5 - bits)) & 31) as usize] as char);
    }
    out
}

fn code_invalid() -> AppError {
    AppError::problem(400, "MFA_CODE_INVALID", "驗證碼錯誤或已使用過，請等下一組")
}

/// 驗證碼正確且時間步比上次新才通過；`pending` = 設定中（尚未啟用）。通過即啟用。
async fn accept_code(st: &AppState, uid: Uuid, code: &str, pending: bool) -> Result<(), AppError> {
    crate::ratelimit::check(&st.pool, &format!("mfa:{uid}"), 5, 300).await?;
    let sealed: Option<Vec<u8>> = sqlx::query_scalar(
        "SELECT totp_secret FROM users WHERE id=$1 AND totp_secret IS NOT NULL AND (totp_enabled_at IS NULL) = $2",
    )
    .bind(uid)
    .bind(pending)
    .fetch_optional(&st.pool)
    .await?;
    let sealed = sealed.ok_or_else(|| {
        AppError::problem(409, "MFA_NOT_SET_UP", "尚未設定兩步驟驗證，請重新設定")
    })?;
    let secret = crate::sealed::open(&sealed).ok_or_else(|| {
        tracing::error!(%uid, "totp_secret 無法解密（SHIPPING_ENC_KEY 變更？）");
        AppError::problem(
            409,
            "MFA_NOT_SET_UP",
            "兩步驟驗證需要重設，請聯絡其他營運人員",
        )
    })?;
    let unix = chrono::Utc::now().timestamp() as u64;
    let step = totp_match(&secret, code.trim(), unix).ok_or_else(code_invalid)?;
    // 條件式更新：同一時間步只能用一次；totp_secret 未被重新 setup 覆寫
    let n = sqlx::query(
        "UPDATE users SET totp_last_step=$2, totp_enabled_at=COALESCE(totp_enabled_at, now())
                         WHERE id=$1 AND totp_secret=$3 AND COALESCE(totp_last_step, -1) < $2",
    )
    .bind(uid)
    .bind(step)
    .bind(&sealed)
    .execute(&st.pool)
    .await?
    .rows_affected();
    if n == 1 {
        Ok(())
    } else {
        Err(code_invalid())
    }
}

async fn mark_session(
    st: &AppState,
    u: &CurrentUser,
    ip: &str,
    action: &str,
) -> Result<(), AppError> {
    let mut tx = st.pool.begin().await?;
    sqlx::query("UPDATE sessions SET mfa_at=now() WHERE id=$1")
        .bind(u.session_id)
        .execute(&mut *tx)
        .await?;
    audit(
        &mut *tx,
        u.id,
        action,
        "sessions",
        u.session_id,
        inet(ip).as_deref(),
        json!({}),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

fn staff_only(u: &CurrentUser) -> Result<(), AppError> {
    if u.is_staff {
        Ok(())
    } else {
        Err(AppError::problem(403, "STAFF_ONLY", "僅營運人員可使用"))
    }
}

/// 產生新密鑰（未啟用前可重複呼叫覆寫）；回傳 base32 與 otpauth URI 供 QR code。
async fn totp_setup(u: CurrentUser, State(st): State<AppState>) -> Result<Json<Value>, AppError> {
    staff_only(&u)?;
    let mut secret = [0u8; 20];
    rand::thread_rng().fill_bytes(&mut secret);
    let email: Option<Option<String>> = sqlx::query_scalar(
        "UPDATE users SET totp_secret=$2, totp_last_step=NULL WHERE id=$1 AND totp_enabled_at IS NULL RETURNING email",
    )
    .bind(u.id)
    .bind(crate::sealed::seal(&secret))
    .fetch_optional(&st.pool)
    .await?;
    let Some(email) = email else {
        return Err(AppError::problem(
            409,
            "MFA_ALREADY_ENABLED",
            "已啟用兩步驟驗證",
        ));
    };
    let b32 = base32(&secret);
    let mut uri = reqwest::Url::parse("otpauth://totp/").expect("固定網址");
    uri.set_path(&format!(
        "Wishly:{}",
        email.unwrap_or_else(|| u.id.to_string())
    ));
    uri.query_pairs_mut()
        .append_pair("secret", &b32)
        .append_pair("issuer", "Wishly");
    Ok(Json(
        json!({ "secret": b32, "otpauth_uri": uri.to_string() }),
    ))
}

#[derive(Deserialize)]
struct CodeReq {
    code: String,
}

async fn totp_enable(
    u: CurrentUser,
    State(st): State<AppState>,
    Peer(ip): Peer,
    Json(r): Json<CodeReq>,
) -> Result<StatusCode, AppError> {
    staff_only(&u)?;
    accept_code(&st, u.id, &r.code, true).await?;
    mark_session(&st, &u, &ip, "auth.mfa_enabled").await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn mfa_verify(
    u: CurrentUser,
    State(st): State<AppState>,
    Peer(ip): Peer,
    Json(r): Json<CodeReq>,
) -> Result<StatusCode, AppError> {
    accept_code(&st, u.id, &r.code, false).await?;
    mark_session(&st, &u, &ip, "auth.mfa_verified").await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------- 登入裝置 ----------
async fn list_sessions(
    u: CurrentUser,
    State(st): State<AppState>,
) -> Result<Json<Value>, AppError> {
    let data: Value = sqlx::query_scalar(
        "SELECT COALESCE(jsonb_agg(jsonb_build_object('id', id, 'user_agent', user_agent, 'ip', host(ip),
                  'created_at', created_at, 'last_seen_at', last_seen_at, 'current', id = $2) ORDER BY last_seen_at DESC), '[]')
         FROM sessions WHERE user_id=$1 AND revoked_at IS NULL AND expires_at > now()",
    )
    .bind(u.id)
    .bind(u.session_id)
    .fetch_one(&st.pool)
    .await?;
    Ok(Json(json!({ "data": data })))
}

async fn revoke_session(
    u: CurrentUser,
    State(st): State<AppState>,
    Peer(ip): Peer,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    let mut tx = st.pool.begin().await?;
    let n = sqlx::query("UPDATE sessions SET revoked_at=now() WHERE id=$1 AND user_id=$2 AND revoked_at IS NULL AND expires_at > now()")
        .bind(id).bind(u.id).execute(&mut *tx).await?.rows_affected();
    if n == 0 {
        return Err(AppError::NotFound);
    }
    audit(
        &mut *tx,
        u.id,
        "auth.session_revoked",
        "sessions",
        id,
        inet(&ip).as_deref(),
        json!({}),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// 登出目前以外的所有裝置
async fn revoke_others(
    u: CurrentUser,
    State(st): State<AppState>,
    Peer(ip): Peer,
) -> Result<Json<Value>, AppError> {
    let mut tx = st.pool.begin().await?;
    let n = revoke_except(&mut tx, u.id, u.session_id).await?;
    audit(
        &mut *tx,
        u.id,
        "auth.sessions_revoked",
        "users",
        u.id,
        inet(&ip).as_deref(),
        json!({ "count": n }),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({ "revoked": n })))
}

async fn revoke_except(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    uid: Uuid,
    keep: Uuid,
) -> Result<u64, AppError> {
    Ok(sqlx::query(
        "UPDATE sessions SET revoked_at=now() WHERE user_id=$1 AND id<>$2 AND revoked_at IS NULL",
    )
    .bind(uid)
    .bind(keep)
    .execute(&mut **tx)
    .await?
    .rows_affected())
}

// ---------- 改密碼 ----------
#[derive(Deserialize)]
struct PasswordReq {
    current_password: String,
    new_password: String,
}

/// 需目前密碼；沒有密碼者走「忘記密碼」（Email OTP）設定，避免被盜的 session 自行加上密碼取得長期登入。
/// 成功後登出其他裝置並寄安全通知。
async fn change_password(
    u: CurrentUser,
    State(st): State<AppState>,
    Peer(ip): Peer,
    Json(r): Json<PasswordReq>,
) -> Result<StatusCode, AppError> {
    crate::auth_ext::check_pw(&r.new_password)
        .map_err(|_| AppError::invalid("/new_password", "RANGE", "密碼長度須為 8–128 字元"))?;
    crate::ratelimit::check(&st.pool, &format!("pw-change:{}", u.id), 5, 900).await?;
    let row: Option<(String, Option<String>)> = sqlx::query_as(
        "SELECT provider_uid, password_hash FROM auth_identities WHERE user_id=$1 AND provider='email'",
    )
    .bind(u.id)
    .fetch_optional(&st.pool)
    .await?;
    let Some((email, Some(old))) = row else {
        return Err(AppError::problem(
            409,
            "PASSWORD_NOT_SET",
            "尚未設定密碼，請使用「設定／重設密碼」以 Email 驗證後設定",
        ));
    };
    let (cur, h) = (r.current_password, old.clone());
    let ok = crate::auth_ext::argon_run(move || {
        cur.chars().count() <= 128 && crate::auth_ext::verify_pw(&cur, &h)
    })
    .await?;
    if !ok {
        return Err(AppError::invalid(
            "/current_password",
            "MISMATCH",
            "目前密碼錯誤",
        ));
    }
    let new = crate::auth_ext::hash_blocking(r.new_password).await?;
    let mut tx = st.pool.begin().await?;
    // 以舊雜湊為條件：驗證期間被重設 / 解除綁定就不覆寫
    let n = sqlx::query("UPDATE auth_identities SET password_hash=$3 WHERE user_id=$1 AND provider='email' AND password_hash=$2")
        .bind(u.id).bind(&old).bind(&new).execute(&mut *tx).await?.rows_affected();
    if n != 1 {
        return Err(AppError::problem(
            409,
            "PASSWORD_CHANGED",
            "密碼剛被變更，請重新操作",
        ));
    }
    revoke_except(&mut tx, u.id, u.session_id).await?;
    sqlx::query("DELETE FROM login_failures WHERE email=$1")
        .bind(&email)
        .execute(&mut *tx)
        .await?;
    crate::identities::notify(
        &mut tx,
        u.id,
        "auth.password_changed",
        "email",
        Some(&email),
    )
    .await?;
    audit(
        &mut *tx,
        u.id,
        "auth.password_changed",
        "users",
        u.id,
        inet(&ip).as_deref(),
        json!({}),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc6238_vectors_and_base32() {
        let k = b"12345678901234567890";
        // RFC 6238 附錄 B（SHA-1）取後 6 碼
        assert_eq!(hotp(k, 59 / 30), 287082);
        assert_eq!(hotp(k, 1111111109 / 30), 81804);
        assert_eq!(totp_match(k, "081804", 1111111109), Some(1111111109 / 30));
        assert_eq!(
            totp_match(k, "081804", 1111111109 + 30),
            Some(1111111109 / 30),
            "容忍前一步"
        );
        assert_eq!(totp_match(k, "081804", 1111111109 + 90), None);
        assert_eq!(totp_match(k, "08180a", 1111111109), None);
        assert_eq!(base32(b"foobar"), "MZXW6YTBOI");
        assert_eq!(base32(&[0u8; 20]).len(), 32);
    }
}
