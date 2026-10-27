//! 最小 Postgres 固定視窗限流 + 每小時清除 task（idempotency_keys 24h、rate_limits 2 天、login_failures 365 天）。
use crate::error::AppError;
use sqlx::PgPool;

/// 計數 +1；回 Ok(true)=未超限，Ok(false)=超限（Err(secs) 以 allow_retry 取得）
async fn hit<'e>(
    pool: impl sqlx::PgExecutor<'e>,
    key: &str,
    limit: i32,
    window_secs: i64,
) -> Result<(bool, i64), AppError> {
    let (count, retry): (i32, i64) = sqlx::query_as(
        "WITH w AS (SELECT to_timestamp(floor(extract(epoch FROM now()) / $2) * $2) AS s)
         INSERT INTO rate_limits (key, window_start, count) SELECT $1, w.s, 1 FROM w
         ON CONFLICT (key, window_start) DO UPDATE SET count = rate_limits.count + 1
         RETURNING count, GREATEST(1, ceil(extract(epoch FROM window_start + make_interval(secs => $2) - now()))::bigint)")
        .bind(key).bind(window_secs as f64).fetch_one(pool).await?;
    Ok((count <= limit, retry))
}

/// 超限 → 429 RATE_LIMITED + Retry-After
pub async fn check<'e>(
    pool: impl sqlx::PgExecutor<'e>,
    key: &str,
    limit: i32,
    window_secs: i64,
) -> Result<(), AppError> {
    match hit(pool, key, limit, window_secs).await? {
        (true, _) => Ok(()),
        (false, r) => Err(AppError::Problem {
            status: 429,
            code: "RATE_LIMITED",
            detail: "請求過於頻繁，請稍後再試".into(),
            errors: None,
            retry_after: Some(r as u64),
        }),
    }
}

/// 不報錯版本：超限回 false（用於「超限就略過副作用」，如不寄信）
pub async fn allow<'e>(
    pool: impl sqlx::PgExecutor<'e>,
    key: &str,
    limit: i32,
    window_secs: i64,
) -> Result<bool, AppError> {
    Ok(hit(pool, key, limit, window_secs).await?.0)
}

pub async fn cleanup(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM idempotency_keys WHERE created_at < now() - interval '24 hours'")
        .execute(pool)
        .await?;
    sqlx::query("DELETE FROM native_auth_codes WHERE expires_at < now()")
        .execute(pool)
        .await?;
    // 過期或撤銷的 session 保留 30 天供裝置列表/稽核對照後清除
    sqlx::query("DELETE FROM sessions WHERE expires_at < now() - interval '30 days'")
        .execute(pool)
        .await?;
    sqlx::query("DELETE FROM rate_limits WHERE window_start < now() - interval '2 days'")
        .execute(pool)
        .await?;
    // 鎖定只看 15 分鐘內，但營運分析最長查 365 天；超出分析範圍的紀錄才清除
    // 被取消而未定案的登入預約到期後不再計入，一併清掉
    sqlx::query("DELETE FROM login_failures WHERE created_at < now() - interval '365 days' OR pending_until < now() - interval '1 hour'")
        .execute(pool)
        .await?;
    // 每日活躍紀錄只留 400 天
    sqlx::query(
        "DELETE FROM user_activity_days WHERE day < (now() AT TIME ZONE 'Asia/Taipei')::date - 400",
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// 每 5 分鐘釋放逾期捐助；每 12 個 tick（1 小時）清除過期資料
pub fn spawn_cleanup(pool: PgPool) {
    tokio::spawn(async move {
        let mut t = tokio::time::interval(std::time::Duration::from_secs(300));
        let mut n = 0u32;
        loop {
            t.tick().await;
            // 先關到期清單（給捐助結束寬限），再釋放逾期捐助
            if let Err(e) = crate::wishlists::close_due(&pool).await {
                tracing::error!(error=%e, "close_due");
            }
            if let Err(e) = crate::claims::expire_due(&pool).await {
                tracing::error!(error=%e, "expire_due");
            }
            if let Err(e) = crate::wishlists::remind_closing(&pool).await {
                tracing::error!(error=%e, "remind_closing");
            }
            if let Err(e) = crate::contributions::tick_funding(&pool).await {
                tracing::error!(error=?e, "tick_funding");
            }
            if n.is_multiple_of(12) {
                if let Err(e) = cleanup(&pool).await {
                    tracing::error!(error=%e, "cleanup");
                }
            }
            n = n.wrapping_add(1);
        }
    });
}

/// 連線對端位址（需 into_make_service_with_connect_info；測試 oneshot 時為 None）
/// 客戶端 IP（依 TRUSTED_PROXY：預設連線對端，cloudflare 時信任 CF-Connecting-IP；從不信任 X-Forwarded-For）
pub struct Peer(pub String);

impl<S: Send + Sync> axum::extract::FromRequestParts<S> for Peer {
    type Rejection = std::convert::Infallible;
    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _: &S,
    ) -> Result<Self, Self::Rejection> {
        Ok(Peer(crate::config::client_ip(parts, &crate::config::get())))
    }
}
