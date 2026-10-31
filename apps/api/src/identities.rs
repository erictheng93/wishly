//! 手動連結登入方式；絕不搬移既有帳號的資料（撞到空帳號時併入後刪除，見 absorb）。所有增刪都鎖 users 列。
use crate::{
    auth,
    error::AppError,
    session::{hash_token, session_token, CurrentUser},
    AppState,
};
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{delete, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/me/identities/email/request", post(request_email))
        .route("/me/identities/email/verify", post(verify_email))
        .route("/me/identities/{provider}", delete(unlink))
}

pub(crate) fn in_use() -> AppError {
    AppError::problem(
        409,
        "IDENTITY_IN_USE",
        "此登入方式或 Email 已屬於另一個有資料的帳號，無法自動合併。請使用原帳號登入，或聯絡客服並提供兩邊的帳號 ID。",
    )
}

pub(crate) fn email_mismatch() -> AppError {
    AppError::problem(
        409,
        "EMAIL_MISMATCH",
        "請綁定目前的聯絡 Email，或先解除提供該 Email 的登入方式。",
    )
}

/// 與自動連結共用鎖順序：identity / email → user，避免競爭及反向鎖死。
pub(crate) async fn lock_identity(
    tx: &mut Transaction<'_, Postgres>,
    provider: &str,
    uid: &str,
    email: Option<&str>,
) -> Result<(), AppError> {
    let mut keys = vec![format!("identity:{provider}:{uid}")];
    if let Some(e) = email {
        keys.push(format!("email:{e}"));
    }
    keys.sort();
    for key in keys {
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(key)
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
}

async fn lock_user(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    session_hash: &[u8],
) -> Result<Option<String>, AppError> {
    let (email, blocked): (Option<String>, bool) = sqlx::query_as("SELECT email, blocked_at IS NOT NULL FROM users WHERE id=$1 AND deleted_at IS NULL FOR UPDATE")
        .bind(id).fetch_optional(&mut **tx).await?.ok_or(AppError::Unauthorized)?;
    if blocked {
        return Err(AppError::problem(
            403,
            "ACCOUNT_BLOCKED",
            "帳號已停權，請聯絡客服",
        ));
    }
    let live: Option<Uuid> = sqlx::query_scalar("SELECT id FROM sessions WHERE user_id=$1 AND token_hash=$2 AND revoked_at IS NULL AND expires_at > now() FOR SHARE")
        .bind(id).bind(session_hash).fetch_optional(&mut **tx).await?;
    if live.is_none() {
        return Err(AppError::Unauthorized);
    }
    Ok(email)
}

/// 保留寄送當下的收件人，即使之後解除 email 也能收到安全信；不受通知偏好影響（Account 類）。
pub(crate) async fn notify(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    kind: &str,
    provider: &str,
    to: Option<&str>,
) -> Result<(), AppError> {
    if let Some(to) = to {
        sqlx::query(
            "INSERT INTO notifications (user_id, channel, kind, payload) VALUES ($1,'email',$2,$3)",
        )
        .bind(id)
        .bind(kind)
        .bind(json!({"provider":provider,"to":to}))
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

pub(crate) async fn link(
    pool: &PgPool,
    id: Uuid,
    session_hash: &[u8],
    provider: &str,
    uid: &str,
    email: Option<&str>,
) -> Result<(), AppError> {
    let mut tx = pool.begin().await?;
    let read_only: Option<Value> =
        sqlx::query_scalar("SELECT value FROM system_flags WHERE key='read_only'")
            .fetch_optional(&mut *tx)
            .await?;
    if read_only.as_ref().and_then(Value::as_bool) == Some(true) {
        return Err(AppError::problem(
            503,
            "READ_ONLY_MODE",
            "系統維護中，稍後再試",
        ));
    }
    lock_identity(&mut tx, provider, uid, email).await?;
    let contact = lock_user(&mut tx, id, session_hash).await?;
    let owner: Option<Uuid> = sqlx::query_scalar(
        "SELECT user_id FROM auth_identities WHERE provider=$1::auth_provider AND provider_uid=$2",
    )
    .bind(provider)
    .bind(uid)
    .fetch_optional(&mut *tx)
    .await?;
    if owner == Some(id) {
        return Ok(());
    }
    let others: Vec<Uuid> = sqlx::query_scalar(
        "SELECT user_id FROM auth_identities WHERE provider=$1::auth_provider AND provider_uid=$2
         UNION SELECT id FROM users WHERE email=$3 AND id<>$4
         UNION SELECT user_id FROM auth_identities WHERE email=$3 AND user_id<>$4",
    )
    .bind(provider)
    .bind(uid)
    .bind(email)
    .bind(id)
    .fetch_all(&mut *tx)
    .await?;
    let other = match others[..] {
        [] => None,
        [other] => Some(other),
        _ => return Err(in_use()),
    };
    if let Some(other) = other {
        lock_blank(&mut tx, other, provider, uid, email).await?;
    }
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM auth_identities WHERE user_id=$1 AND provider=$2::auth_provider)")
        .bind(id).bind(provider).fetch_one(&mut *tx).await?;
    if exists {
        return Err(AppError::problem(
            409,
            "PROVIDER_ALREADY_LINKED",
            "已綁定此登入方式，請先解除後再更換。",
        ));
    }
    // Email 登入與通知使用同一信箱，避免解除後重設密碼又隱性綁回。
    if provider == "email" && contact.as_deref().is_some_and(|c| Some(c) != email) {
        return Err(email_mismatch());
    }
    let (media, moved) = match other {
        Some(other) => absorb(&mut tx, id, other, provider, uid).await?,
        None => (vec![], false),
    };
    if !moved {
        sqlx::query("INSERT INTO auth_identities (user_id,provider,provider_uid,email) VALUES ($1,$2::auth_provider,$3,$4)")
            .bind(id).bind(provider).bind(uid).bind(email).execute(&mut *tx).await?;
    }
    sqlx::query("UPDATE users SET email=coalesce(email,$2) WHERE id=$1")
        .bind(id)
        .bind(email)
        .execute(&mut *tx)
        .await?;
    let to = contact.as_deref().or(email);
    notify(&mut tx, id, "auth.identity_linked", provider, to).await?;
    tx.commit().await?;
    crate::uploads::delete_objects(pool.clone(), media); // COMMIT 後才刪
    Ok(())
}

/// 綁定撞到另一個帳號：鎖住它並確認仍衝突且為「空帳號」（只有登入方式，見 user_is_blank），否則 IDENTITY_IN_USE。
/// 已持有本人 users 列：對方的鎖一律不等（try / SKIP LOCKED），避免兩個帳號互相吸收時鎖死。
async fn lock_blank(
    tx: &mut Transaction<'_, Postgres>,
    other: Uuid,
    provider: &str,
    uid: &str,
    email: Option<&str>,
) -> Result<(), AppError> {
    let busy = || AppError::problem(409, "IDENTITY_BUSY", "另一個帳號正在處理中，請稍後再試。");
    if !crate::uploads::try_lock_user(tx, other).await? {
        return Err(busy());
    }
    let locked: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM users WHERE id=$1 FOR UPDATE SKIP LOCKED")
            .bind(other)
            .fetch_optional(&mut **tx)
            .await?;
    // 衝突是在鎖住對方前讀到的：對方可能剛解除那個登入方式，重新核對
    let still: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM auth_identities WHERE user_id=$1 AND ((provider=$2::auth_provider AND provider_uid=$3) OR email=$4)) OR EXISTS(SELECT 1 FROM users WHERE id=$1 AND email=$4)")
        .bind(other).bind(provider).bind(uid).bind(email).fetch_one(&mut **tx).await?;
    if locked.is_none() || !still {
        return Err(busy());
    }
    let blank: bool = sqlx::query_scalar("SELECT user_is_blank($1)")
        .bind(other)
        .fetch_one(&mut **tx)
        .await?;
    if !blank {
        return Err(in_use());
    }
    Ok(())
}

/// 刪掉 lock_blank 鎖住的空帳號，並把這次綁定的 identity 搬過來（保留密碼）；不搬任何資料。
/// 回傳待刪的媒體與 identity 是否已搬移。
async fn absorb(
    tx: &mut Transaction<'_, Postgres>,
    into: Uuid,
    other: Uuid,
    provider: &str,
    uid: &str,
) -> Result<(Vec<String>, bool), AppError> {
    sqlx::query("UPDATE sessions SET revoked_at=now() WHERE user_id=$1 AND revoked_at IS NULL")
        .bind(other)
        .execute(&mut **tx)
        .await?;
    for sql in [
        "DELETE FROM user_activity_days WHERE user_id=$1",
        "DELETE FROM native_auth_codes WHERE user_id=$1",
        "DELETE FROM otp_challenges WHERE link_user_id=$1",
        "UPDATE notifications SET status='cancelled' WHERE user_id=$1 AND status='pending'",
        "UPDATE notifications SET payload=payload - 'to' WHERE user_id=$1 AND kind IN ('auth.identity_linked','auth.identity_unlinked')",
        "UPDATE notifications SET inbox_at=NULL, inbox_title=NULL, inbox_link=NULL WHERE user_id=$1",
    ] {
        sqlx::query(sql).bind(other).execute(&mut **tx).await?;
    }
    sqlx::query("DELETE FROM auth_identities WHERE user_id=$1 AND NOT (provider=$2::auth_provider AND provider_uid=$3)")
        .bind(other).bind(provider).bind(uid).execute(&mut **tx).await?;
    let moved = sqlx::query("UPDATE auth_identities SET user_id=$2 WHERE user_id=$1")
        .bind(other)
        .bind(into)
        .execute(&mut **tx)
        .await?
        .rows_affected()
        > 0;
    // 空帳號只可能有頭像（OAuth 轉存）；與刪帳號相同，同交易要求退役、COMMIT 後才刪物件。
    let media: Vec<String> = sqlx::query_scalar(
        "SELECT object_key FROM user_media WHERE user_id=$1
         UNION SELECT avatar_key FROM users WHERE id=$1 AND avatar_key IS NOT NULL",
    )
    .bind(other)
    .fetch_all(&mut **tx)
    .await?;
    let media: Vec<String> = media
        .into_iter()
        .filter(|k| crate::uploads::avatar_url(Some(k)).is_some())
        .collect();
    for key in &media {
        sqlx::query(
            "INSERT INTO user_media (user_id, object_key) VALUES ($1,$2) ON CONFLICT DO NOTHING",
        )
        .bind(other)
        .bind(key)
        .execute(&mut **tx)
        .await?;
    }
    crate::uploads::request_retire(tx, &media).await?;
    // 先清掉 email（UNIQUE），呼叫者才能把同一信箱寫到本人帳號
    sqlx::query(
        "UPDATE users SET display_name='已刪除的使用者', email=NULL, avatar_key=NULL, bio=NULL, birthday=NULL, notification_prefs='{}'::jsonb,
                deleted_at=now(), anonymized_at=now() WHERE id=$1",
    )
    .bind(other)
    .execute(&mut **tx)
    .await?;
    sqlx::query("INSERT INTO audit_logs (actor_type, actor_id, action, entity, entity_id, diff) VALUES ('user', $1, 'account.absorb', 'users', $2, jsonb_build_object('provider', $3::text))")
        .bind(into).bind(other).bind(provider).execute(&mut **tx).await?;
    Ok((media, moved))
}

#[derive(Deserialize)]
struct Email {
    email: String,
}

fn session_hash(headers: &HeaderMap) -> Result<Vec<u8>, AppError> {
    session_token(headers)
        .map(|t| hash_token(&t))
        .ok_or(AppError::Unauthorized)
}

async fn request_email(
    State(st): State<AppState>,
    u: CurrentUser,
    headers: HeaderMap,
    Json(b): Json<Email>,
) -> Result<(StatusCode, Json<Value>), AppError> {
    let email = auth::norm_email(&b.email)?;
    crate::ratelimit::check(&st.pool, &format!("identity-email:{}", u.id), 5, 3600).await?;
    auth::issue_otp_scoped(
        &st.pool,
        &email,
        "link",
        None,
        None,
        Some(u.id),
        Some(&session_hash(&headers)?),
        false,
    )
    .await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({"expires_in":600,"resend_after":60})),
    ))
}

async fn verify_email(
    State(st): State<AppState>,
    u: CurrentUser,
    headers: HeaderMap,
    Json(b): Json<auth::VerifyReq>,
) -> Result<Json<Value>, AppError> {
    let email = auth::norm_email(&b.email)?;
    let hash = session_hash(&headers)?;
    auth::consume_otp_scoped(
        &st.pool,
        &email,
        &b.code,
        "link",
        Some(u.id),
        Some(&hash),
        None,
    )
    .await?;
    link(&st.pool, u.id, &hash, "email", &email, Some(&email)).await?;
    Ok(Json(auth::user_json(&st.pool, u.id).await?))
}

async fn unlink(
    State(st): State<AppState>,
    u: CurrentUser,
    headers: HeaderMap,
    Path(provider): Path<String>,
) -> Result<Json<Value>, AppError> {
    if !["email", "google", "line"].contains(&provider.as_str()) {
        return Err(AppError::invalid("/provider", "ENUM", "不支援此登入方式"));
    }
    let mut tx = st.pool.begin().await?;
    let hash = session_hash(&headers)?;
    let contact = lock_user(&mut tx, u.id, &hash).await?;
    unlink_locked(&mut tx, u.id, &provider, contact.as_deref()).await?;
    sqlx::query("UPDATE sessions SET revoked_at=now() WHERE user_id=$1 AND token_hash<>$2 AND revoked_at IS NULL")
        .bind(u.id).bind(&hash).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(Json(auth::user_json(&st.pool, u.id).await?))
}

/// 呼叫者已鎖住 users 列；自助與營運解除綁定共用最後登入方式、聯絡 Email 與安全通知規則。
pub(crate) async fn unlink_locked(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    provider: &str,
    contact: Option<&str>,
) -> Result<(), AppError> {
    let providers: Vec<String> =
        sqlx::query_scalar("SELECT provider::text FROM auth_identities WHERE user_id=$1")
            .bind(id)
            .fetch_all(&mut **tx)
            .await?;
    if !providers.iter().any(|p| p == provider) {
        return Err(AppError::NotFound);
    }
    if providers.iter().all(|p| p == provider) {
        return Err(AppError::problem(
            409,
            "LAST_IDENTITY",
            "至少須保留一種登入方式，請先新增其他方式。",
        ));
    }
    sqlx::query("DELETE FROM auth_identities WHERE user_id=$1 AND provider=$2::auth_provider")
        .bind(id)
        .bind(provider)
        .execute(&mut **tx)
        .await?;
    let remaining: Option<String> = sqlx::query_scalar("SELECT email FROM auth_identities WHERE user_id=$1 AND email IS NOT NULL ORDER BY (email=$2) DESC, created_at LIMIT 1")
        .bind(id).bind(contact).fetch_optional(&mut **tx).await?;
    notify(tx, id, "auth.identity_unlinked", provider, contact).await?;
    sqlx::query("UPDATE users SET email=$2 WHERE id=$1")
        .bind(id)
        .bind(remaining)
        .execute(&mut **tx)
        .await?;
    Ok(())
}
