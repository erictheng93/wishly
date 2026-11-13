//! 營運後台讀取端的保護：列表／聚合查詢的 statement_timeout，以及聚合結果的 60 秒程序內快取。
//! 只給 staff 端點用（多一次往返無妨）；不可設成連線池全域，否則 migration 與背景工作也會被中斷。
use crate::points::Tx;
use serde_json::Value;
use sqlx::PgPool;
use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

pub const STMT_TIMEOUT: &str = "8s";
const CACHE_TTL: Duration = Duration::from_secs(60);
const CACHE_MAX: usize = 128;

/// 開交易並設本交易的 statement_timeout；逾時（57014）由 `error.rs` 對應為暫時性壅塞回應。
pub async fn begin_read(pool: &PgPool) -> Result<Tx<'static>, sqlx::Error> {
    begin_read_with(pool, STMT_TIMEOUT).await
}

pub async fn begin_read_with(pool: &PgPool, timeout: &str) -> Result<Tx<'static>, sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('statement_timeout', $1, true)")
        .bind(timeout)
        .execute(&mut *tx)
        .await?;
    Ok(tx)
}

type Cache = Mutex<Option<HashMap<String, (Instant, Value)>>>;
static CACHE: Cache = Mutex::new(None);

/// 快取最終 JSON；鍵由呼叫端以「端點＋已驗證的參數」組成。須在角色守衛通過之後才呼叫。
/// `micro_cache` 關閉（測試）時直接執行 `f`。
pub async fn cached<F, Fut, E>(key: String, f: F) -> Result<Value, E>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<Value, E>>,
{
    if !crate::config::get().micro_cache {
        return f().await;
    }
    if let Some((at, v)) = CACHE
        .lock()
        .unwrap()
        .get_or_insert_with(HashMap::new)
        .get(&key)
    {
        if at.elapsed() < CACHE_TTL {
            return Ok(v.clone());
        }
    }
    let v = f().await?;
    let mut g = CACHE.lock().unwrap();
    let m = g.get_or_insert_with(HashMap::new);
    m.retain(|_, (at, _)| at.elapsed() < CACHE_TTL);
    if m.len() >= CACHE_MAX {
        m.clear(); // 鍵來自已驗證的 days，正常遠低於上限；保險用
    }
    m.insert(key, (Instant::now(), v.clone()));
    Ok(v)
}
