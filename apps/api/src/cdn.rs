//! CDN 快取清除（Cloudflare purge by URL，所有方案皆可用）。
//! GET /public/wishlists/{slug} 對 public 清單回 s-maxage=10 + stale-while-revalidate=60，邊緣可能留舊副本約一分鐘；
//! 清單「不再能公開提供」時（收緊成 friends / selected / private、封存、下架、刪帳號）立刻清掉，
//! 避免剛設成好友限定的清單仍被拿到連結的人看到。
//! 未設 CF_ZONE_ID + CF_CACHE_PURGE_TOKEN（dev / test）時不做事；production 經 Cloudflare 時啟動即要求（config.rs）。
use crate::config::Config;
use std::time::Duration;

/// 公開（可快取）路徑是否會提供這份清單的內容（與 public.rs render 的匿名判斷一致）
pub fn servable(visibility: &str, status: &str, moderation: &str) -> bool {
    visibility == "public" && matches!(status, "active" | "closed") && moderation == "ok"
}

fn public_url(cfg: &Config, slug: &str) -> String {
    format!(
        "{}/api/v1/public/wishlists/{slug}",
        cfg.api_base_url.trim_end_matches('/')
    )
}

/// 一次清除多個 URL；錯誤字串不含 token
pub async fn purge(cfg: &Config, urls: &[String]) -> Result<(), String> {
    purge_body(cfg, &serde_json::json!({ "files": urls })).await
}

async fn purge_body(cfg: &Config, body: &serde_json::Value) -> Result<(), String> {
    let (Some(zone), Some(tok)) = (&cfg.cf_zone_id, &cfg.cf_purge_token) else {
        return Ok(());
    };
    let url = format!(
        "{}/zones/{zone}/purge_cache",
        cfg.cf_api_base.trim_end_matches('/')
    );
    let r = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| e.without_url().to_string())?
        .post(url)
        .bearer_auth(tok)
        .json(body)
        .send()
        .await
        .map_err(|e| e.without_url().to_string())?;
    let status = r.status();
    let v: serde_json::Value = r.json().await.unwrap_or_default();
    if status.is_success() && v["success"] == true {
        Ok(())
    } else {
        Err(format!("cloudflare purge {status}: {}", v["errors"]))
    }
}

/// 背景清除（不阻塞回應），每批最多 100 個 URL；失敗重試 3 次（1s、4s、16s）後只記 log。
/// ponytail: 程序重啟會遺失尚未成功的清除；圖片可能保留到快取過期，需保證送達時改成 DB 佇列。
fn purge_urls(cfg: Config, field: &'static str, urls: Vec<String>) {
    if urls.is_empty() || cfg.cf_zone_id.is_none() || cfg.cf_purge_token.is_none() {
        return;
    }
    tokio::spawn(async move {
        for batch in urls.chunks(100) {
            for (i, wait) in [1u64, 4, 16, 0].into_iter().enumerate() {
                match purge_body(&cfg, &serde_json::json!({ (field): batch })).await {
                    Ok(()) => break,
                    Err(e) if wait == 0 => {
                        tracing::error!(error = %e, ?batch, "CDN 清除失敗，已放棄")
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, attempt = i + 1, "CDN 清除失敗，重試");
                        tokio::time::sleep(Duration::from_secs(wait)).await;
                    }
                }
            }
        }
    });
}

pub fn purge_wishlists(slugs: Vec<String>) {
    let cfg = crate::config::get();
    let urls = slugs.iter().map(|s| public_url(&cfg, s)).collect();
    purge_urls(cfg, "files", urls);
}

/// 事件公開頁（GET /public/relief-events/{slug}）
pub fn purge_events(slugs: Vec<String>) {
    let cfg = crate::config::get();
    let base = cfg.api_base_url.trim_end_matches('/').to_string();
    let urls = slugs.iter().map(|s| format!("{base}/api/v1/public/relief-events/{}", s.trim())).collect();
    purge_urls(cfg, "files", urls);
}

/// 在 R2 DELETE 成功或 404 後呼叫，避免清除快取後又從尚未刪除的 origin 回填。
pub fn purge_media(keys: Vec<String>) {
    let cfg = crate::config::get();
    // Full object-path prefixes also remove query-string and custom-cache-key variants.
    let prefixes = keys
        .iter()
        .filter_map(|key| {
            let url = reqwest::Url::parse(&cfg.s3.public_url(key)).ok()?;
            let host = url.host_str()?;
            let port = url.port().map(|p| format!(":{p}")).unwrap_or_default();
            Some(format!("{host}{port}{}", url.path()))
        })
        .collect();
    purge_urls(cfg, "prefixes", prefixes);
}

#[cfg(test)]
mod tests {
    use super::servable;
    #[test]
    fn servable_only_public_live_ok() {
        assert!(servable("public", "active", "ok") && servable("public", "closed", "ok"));
        for v in ["friends", "selected", "link", "password", "private"] {
            assert!(!servable(v, "active", "ok"), "{v}");
        }
        assert!(
            !servable("public", "archived", "ok")
                && !servable("public", "draft", "ok")
                && !servable("public", "active", "hidden")
        );
    }
}
