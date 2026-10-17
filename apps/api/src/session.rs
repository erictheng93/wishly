//! 共用身分：Cookie `ws_session=<opaque token>`，DB 只存 SHA-256(token)（sessions.token_hash）。
//! 簽發 session 的是 auth 模組；其他模組只用 `CurrentUser` extractor。
use crate::{error::AppError, AppState};
use axum::{
    extract::FromRequestParts,
    http::{header, request::Parts},
};
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub fn hash_token(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

pub fn cookie_value<'a>(parts: &'a Parts, name: &str) -> Option<&'a str> {
    parts
        .headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .find_map(|c| c.trim().strip_prefix(name)?.strip_prefix('='))
}

/// 登入權杖：先 cookie `ws_session`（網頁），否則 `Authorization: Bearer`（Tauri / 原生，不帶 cookie）。
pub fn session_token(h: &axum::http::HeaderMap) -> Option<String> {
    crate::auth::cookie(h, "ws_session").or_else(|| {
        let t = h
            .get(header::AUTHORIZATION)?
            .to_str()
            .ok()?
            .strip_prefix("Bearer ")?
            .trim();
        (!t.is_empty()).then(|| t.to_string())
    })
}

pub struct CurrentUser {
    pub id: Uuid,
    pub is_staff: bool,
    pub session_id: Uuid,
    /// 此 session 已通過 TOTP（Staff 守衛用）
    pub mfa: bool,
    /// 帳號已啟用 TOTP
    pub mfa_enabled: bool,
}

#[derive(sqlx::FromRow)]
struct SessionRow {
    id: Uuid,
    is_staff: bool,
    blocked: bool,
    live: bool,
    session_id: Uuid,
    mfa: bool,
    mfa_enabled: bool,
    stale: bool,
}

impl FromRequestParts<AppState> for CurrentUser {
    type Rejection = AppError;
    async fn from_request_parts(parts: &mut Parts, st: &AppState) -> Result<Self, AppError> {
        let tok = session_token(&parts.headers).ok_or(AppError::Unauthorized)?;
        let row: Option<SessionRow> = sqlx::query_as(
            "SELECT u.id, u.is_staff, u.blocked_at IS NOT NULL AS blocked, s.revoked_at IS NULL AS live, s.id AS session_id,
                    s.mfa_at IS NOT NULL AS mfa, u.totp_enabled_at IS NOT NULL AS mfa_enabled,
                    (s.last_seen_at < now() - interval '5 minutes' OR NOT EXISTS (SELECT 1 FROM user_activity_days a WHERE a.user_id = u.id AND a.day = (now() AT TIME ZONE 'Asia/Taipei')::date)) AS stale
             FROM sessions s JOIN users u ON u.id = s.user_id
             WHERE s.token_hash = $1 AND s.expires_at > now() AND u.deleted_at IS NULL")
            .bind(hash_token(&tok)).fetch_optional(&st.pool).await?;
        match row {
            Some(r) if r.blocked => Err(AppError::problem(
                403,
                "ACCOUNT_BLOCKED",
                "帳號已停權，請聯絡客服",
            )),
            Some(r) if r.live => {
                // 裝置列表的「最後使用」與每日活躍紀錄（台北日）：登入後首個請求、跨日首個請求、之後每 5 分鐘最多寫一次；
                // 失敗不影響請求。先寫活躍列再更新 session：FK 會對 users 取 KEY SHARE，若與持有 users 鎖的交易
                // （刪帳、停權）交錯會死結；SKIP LOCKED 讓被鎖時直接略過（下個請求因 stale 仍成立而補寫）。
                if r.stale {
                    let done = sqlx::query(
                        "INSERT INTO user_activity_days (user_id, day)
                         SELECT id, (now() AT TIME ZONE 'Asia/Taipei')::date FROM users WHERE id = $1
                         FOR KEY SHARE SKIP LOCKED
                         ON CONFLICT DO NOTHING")
                        .bind(r.id)
                        .execute(&st.pool)
                        .await;
                    if done.is_ok() {
                        let _ =
                            sqlx::query("UPDATE sessions SET last_seen_at = now() WHERE id = $1")
                                .bind(r.session_id)
                                .execute(&st.pool)
                                .await;
                    }
                }
                Ok(CurrentUser {
                    id: r.id,
                    is_staff: r.is_staff,
                    session_id: r.session_id,
                    mfa: r.mfa,
                    mfa_enabled: r.mfa_enabled,
                })
            }
            _ => Err(AppError::Unauthorized),
        }
    }
}
