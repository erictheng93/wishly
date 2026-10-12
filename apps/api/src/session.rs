//! 共用身分：Cookie `ws_session=<opaque token>`，DB 只存 SHA-256(token)（sessions.token_hash）。
//! 簽發 session 的是 auth 模組；其他模組只用 `CurrentUser` extractor。
use crate::{error::AppError, AppState};
use axum::{extract::FromRequestParts, http::{header, request::Parts}};
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub fn hash_token(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

pub fn cookie_value<'a>(parts: &'a Parts, name: &str) -> Option<&'a str> {
    parts.headers.get(header::COOKIE)?.to_str().ok()?.split(';')
        .find_map(|c| c.trim().strip_prefix(name)?.strip_prefix('='))
}

/// 登入權杖：先 cookie `ws_session`（網頁），否則 `Authorization: Bearer`（Tauri / 原生，不帶 cookie）。
pub fn session_token(h: &axum::http::HeaderMap) -> Option<String> {
    crate::auth::cookie(h, "ws_session").or_else(|| {
        let t = h.get(header::AUTHORIZATION)?.to_str().ok()?.strip_prefix("Bearer ")?.trim();
        (!t.is_empty()).then(|| t.to_string())
    })
}

pub struct CurrentUser { pub id: Uuid, pub is_staff: bool }

impl FromRequestParts<AppState> for CurrentUser {
    type Rejection = AppError;
    async fn from_request_parts(parts: &mut Parts, st: &AppState) -> Result<Self, AppError> {
        let tok = session_token(&parts.headers).ok_or(AppError::Unauthorized)?;
        let row: Option<(Uuid, bool, bool, bool)> = sqlx::query_as(
            "SELECT u.id, u.is_staff, u.blocked_at IS NOT NULL, s.revoked_at IS NULL FROM sessions s JOIN users u ON u.id = s.user_id
             WHERE s.token_hash = $1 AND s.expires_at > now() AND u.deleted_at IS NULL")
            .bind(hash_token(&tok)).fetch_optional(&st.pool).await?;
        match row {
            Some((_, _, true, _)) => Err(AppError::problem(403, "ACCOUNT_BLOCKED", "帳號已停權，請聯絡客服")),
            Some((id, is_staff, false, true)) => Ok(CurrentUser { id, is_staff }),
            _ => Err(AppError::Unauthorized),
        }
    }
}
