//! 共用身分：Cookie `ws_session=<opaque token>`，DB 只存 SHA-256(token)（sessions.token_hash）。
//! 簽發 session 的是 auth 模組；其他模組只用 `CurrentUser` extractor。
//! 期限：閒置 30 天（使用中隨 last_seen_at 一起續期）、自建立起最長 180 天（減少重新登入的驗證碼信）；MFA 通過後 12 小時內有效。
use crate::{error::AppError, AppState};
use axum::{
    extract::FromRequestParts,
    http::{header, request::Parts},
};
use chrono::{DateTime, Utc};
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
    /// 營運角色位元（admin::ADMIN / FINANCE / MODERATOR / SUPPORT）
    pub staff_roles: u8,
    pub session_id: Uuid,
    /// 此 session 已通過 TOTP（Staff 守衛用）
    pub mfa: bool,
    /// 帳號已啟用 TOTP
    pub mfa_enabled: bool,
    /// system_flags.staff_mfa_required：未啟用 TOTP 的營運人員也須先設定
    pub mfa_required: bool,
}

#[derive(sqlx::FromRow)]
struct SessionRow {
    id: Uuid,
    is_staff: bool,
    staff_roles: Vec<String>,
    blocked: bool,
    live: bool,
    session_id: Uuid,
    /// mfa_at + 12 小時：存期限而非布林，快取中的項目才會準時過期
    mfa_until: Option<DateTime<Utc>>,
    mfa_enabled: bool,
    mfa_required: bool,
    stale: bool,
    expires_at: DateTime<Utc>,
}

impl FromRequestParts<AppState> for CurrentUser {
    type Rejection = AppError;
    async fn from_request_parts(parts: &mut Parts, st: &AppState) -> Result<Self, AppError> {
        let tok = session_token(&parts.headers).ok_or(AppError::Unauthorized)?;
        let hash = hash_token(&tok);
        if let Some(a) = cache::get(&hash) {
            return Ok(a.current());
        }
        let gen = cache::generation();
        let row: Option<SessionRow> = sqlx::query_as(
            "SELECT u.id, u.is_staff, u.staff_roles, u.blocked_at IS NOT NULL AS blocked, s.revoked_at IS NULL AS live, s.id AS session_id,
                    s.mfa_at + interval '12 hours' AS mfa_until, u.totp_enabled_at IS NOT NULL AS mfa_enabled,
                    u.is_staff AND COALESCE((SELECT value = 'true'::jsonb FROM system_flags WHERE key = 'staff_mfa_required'), false) AS mfa_required,
                    (s.last_seen_at < now() - interval '5 minutes' OR NOT EXISTS (SELECT 1 FROM user_activity_days a WHERE a.user_id = u.id AND a.day = (now() AT TIME ZONE 'Asia/Taipei')::date)) AS stale,
                    s.expires_at
             FROM sessions s JOIN users u ON u.id = s.user_id
             WHERE s.token_hash = $1 AND s.expires_at > now() AND u.deleted_at IS NULL")
            .bind(&hash).fetch_optional(&st.pool).await?;
        match row {
            Some(r) if r.blocked => Err(AppError::problem(
                403,
                "ACCOUNT_BLOCKED",
                "帳號已停權，請聯絡客服",
            )),
            Some(r) if r.live => {
                // 裝置列表的「最後使用」、閒置期限續期與每日活躍紀錄（台北日）：登入後首個請求、跨日首個請求、之後每 5 分鐘最多寫一次；
                // 失敗不影響請求。先寫活躍列再更新 session：FK 會對 users 取 KEY SHARE，若與持有 users 鎖的交易
                // （刪帳、停權）交錯會死結；SKIP LOCKED 讓被鎖時直接略過（下個請求因 stale 仍成立而補寫）。
                // 快取命中時不寫：快取最多 30 秒，之後回 DB 時 stale 會再判斷一次。
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
                            sqlx::query("UPDATE sessions SET last_seen_at = now(), expires_at = LEAST(created_at + interval '180 days', now() + interval '30 days') WHERE id = $1")
                                .bind(r.session_id)
                                .execute(&st.pool)
                                .await;
                    }
                }
                let a = cache::Auth {
                    id: r.id,
                    is_staff: r.is_staff,
                    staff_roles: crate::admin::role_mask(&r.staff_roles),
                    session_id: r.session_id,
                    mfa_until: r.mfa_until,
                    mfa_enabled: r.mfa_enabled,
                    mfa_required: r.mfa_required,
                    expires_at: r.expires_at,
                };
                let cu = a.current();
                if let Some(g) = gen {
                    cache::put(hash, a, g);
                }
                Ok(cu)
            }
            _ => Err(AppError::Unauthorized),
        }
    }
}

/// 有效登入的 user id（給 `access::viewer` 用）：只查快取，未命中回 None 由呼叫端自己查 DB。
pub(crate) fn cached_user(token: &str) -> Option<Uuid> {
    cache::get(&hash_token(token)).map(|a| a.id)
}

/// 開啟 session 快取：連上 LISTEN session_inval 後才生效；斷線期間快取停用並清空，重連後重新啟用。
/// 只有 `config.session_cache` 為真時啟動（main.rs）。
pub fn spawn_cache_listener(pool: sqlx::PgPool) {
    if !crate::config::get().session_cache {
        return;
    }
    tokio::spawn(async move {
        loop {
            if pool.is_closed() {
                return;
            }
            if let Ok(mut l) = crate::dashboard::pg_listener(&pool).await {
                if l.listen(cache::CHANNEL).await.is_ok() {
                    cache::set_active(true);
                    // try_recv：斷線時回 Ok(None) 而不是悄悄重連（重連期間的通知會遺失），我們必須知道並停用快取
                    while let Ok(Some(_)) = l.try_recv().await {
                        cache::invalidate_all();
                    }
                    cache::set_active(false);
                    tracing::warn!("session 快取的 LISTEN 斷線，快取停用，稍後重連");
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
    });
}

/// 只供測試：LISTEN 是否已連上（快取是否生效）
pub fn cache_active() -> bool {
    cache::is_active()
}

/// 程序內的成功驗證快取。安全性來自失效機制，不是 TTL：
/// - 只快取「有效、未停權」的結果；失敗、停權一律每次查 DB。
/// - DB trigger（0065）在撤銷、MFA、停權、刪除、staff 變更時 NOTIFY，listener 收到即清空並遞增世代。
/// - 查 DB 前記下世代、寫回時世代不同就丟棄：避免在變更 commit 前讀到舊值的請求，在清空後把舊值寫回。
/// - LISTEN 未連上時完全不用快取；TTL 30 秒只是保險。
mod cache {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    pub const CHANNEL: &str = "session_inval";
    const TTL: Duration = Duration::from_secs(30);
    const MAX_ENTRIES: usize = 50_000;

    #[derive(Clone)]
    pub struct Auth {
        pub id: Uuid,
        pub is_staff: bool,
        pub staff_roles: u8,
        pub session_id: Uuid,
        pub mfa_until: Option<DateTime<Utc>>,
        pub mfa_enabled: bool,
        pub mfa_required: bool,
        pub expires_at: DateTime<Utc>,
    }

    impl Auth {
        pub fn current(&self) -> CurrentUser {
            CurrentUser {
                id: self.id,
                is_staff: self.is_staff,
                staff_roles: self.staff_roles,
                session_id: self.session_id,
                mfa: self.mfa_until.is_some_and(|t| t > Utc::now()),
                mfa_enabled: self.mfa_enabled,
                mfa_required: self.mfa_required,
            }
        }
    }

    struct State {
        active: bool,
        gen: u64,
        map: HashMap<Vec<u8>, (Instant, Auth)>,
    }
    static S: Mutex<Option<State>> = Mutex::new(None);

    fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
        let mut g = S.lock().unwrap();
        f(g.get_or_insert_with(|| State {
            active: false,
            gen: 0,
            map: HashMap::new(),
        }))
    }

    pub fn get(hash: &[u8]) -> Option<Auth> {
        with(|s| {
            if !s.active {
                return None;
            }
            let (at, a) = s.map.get(hash)?;
            (at.elapsed() < TTL && a.expires_at > Utc::now()).then(|| a.clone())
        })
    }

    /// 查 DB 前呼叫；None = 快取停用，不要寫回
    pub fn generation() -> Option<u64> {
        with(|s| s.active.then_some(s.gen))
    }

    pub fn put(hash: Vec<u8>, a: Auth, gen: u64) {
        with(|s| {
            if !s.active || s.gen != gen {
                return;
            }
            if s.map.len() >= MAX_ENTRIES {
                s.map.retain(|_, (at, _)| at.elapsed() < TTL);
                if s.map.len() >= MAX_ENTRIES {
                    s.map.clear();
                }
            }
            s.map.insert(hash, (Instant::now(), a));
        })
    }

    pub fn invalidate_all() {
        with(|s| {
            s.gen = s.gen.wrapping_add(1);
            s.map.clear();
        })
    }

    pub fn set_active(on: bool) {
        with(|s| {
            s.active = on;
            s.gen = s.gen.wrapping_add(1);
            s.map.clear();
        })
    }

    pub fn is_active() -> bool {
        with(|s| s.active)
    }
}
