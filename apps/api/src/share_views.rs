//! POST /public/wishlists/{slug}/views：分享頁瀏覽紀錄（未登入訪客）。
//! 公開清單 GET 會被 CDN 快取、不可讀 cookie，所以由前端掛載後另外回報。
//! 一律回 204（不洩漏清單是否存在）；IP 只寫入 share_views，不進任何回應或日誌。
use crate::{error::AppError, ratelimit::Peer, AppState};
use axum::{
    body::Bytes,
    extract::{DefaultBodyLimit, Path, State},
    http::{header, request::Parts, HeaderMap, StatusCode},
    routing::post,
    Router,
};

pub fn routes() -> Router<AppState> {
    // 本文只有兩個短欄位；過大的請求由框架直接回 413
    Router::new().route(
        "/public/wishlists/{slug}/views",
        post(record).layer(DefaultBodyLimit::max(1024)),
    )
}

const BOTS: [&str; 7] = [
    "bot",
    "crawl",
    "spider",
    "facebookexternalhit",
    "preview",
    "slurp",
    "whatsapp",
];

fn device(ua: &str) -> &'static str {
    let u = ua.to_ascii_lowercase();
    if u.contains("ipad")
        || u.contains("tablet")
        || (u.contains("android") && !u.contains("mobile"))
    {
        "tablet"
    } else if u.contains("mobi") || u.contains("iphone") || u.contains("android") {
        "mobile"
    } else {
        "desktop"
    }
}

/// 網域：須含 `.`、只含字母數字 . -，且不是純數字（排除 IP 與 `1.2.3.4` 之類）
fn host_ok(h: &str) -> bool {
    !h.is_empty()
        && h.len() <= 253
        && h.contains('.')
        && h.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
        && !h.bytes().all(|b| b.is_ascii_digit() || b == b'.')
}

/// 國家 / 城市只在信任 Cloudflare 時讀取，否則任何人都能偽造；國家須為兩個字母並轉大寫
fn geo(headers: &HeaderMap, trusted: bool) -> (Option<String>, Option<String>) {
    if !trusted {
        return (None, None);
    }
    let h = |k: &str| {
        headers
            .get(k)
            .and_then(|v| v.to_str().ok())
            .map(|v| v.chars().take(100).collect::<String>())
            .filter(|v| !v.is_empty())
    };
    (
        h("cf-ipcountry")
            .filter(|c| c.len() == 2 && c.bytes().all(|b| b.is_ascii_alphabetic()))
            .map(|c| c.to_ascii_uppercase()),
        h("cf-ipcity"),
    )
}

/// 限流 key：IPv6 取 /64（單一用戶端通常握有整個 /64，逐位址計數形同沒限流）
fn ip_key(ip: std::net::IpAddr) -> String {
    // IPv4-mapped（雙堆疊 socket）要先還原，否則所有 IPv4 訪客會共用同一個 /64 鍵
    match ip.to_canonical() {
        std::net::IpAddr::V4(v) => v.to_string(),
        std::net::IpAddr::V6(v) => format!("{:x}/64", u128::from(v) >> 64),
    }
}

async fn record(
    State(st): State<AppState>,
    Path(slug): Path<String>,
    Peer(ip): Peer,
    parts: Parts,
    body: Bytes,
) -> Result<StatusCode, AppError> {
    let ua = parts
        .headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .trim();
    let lua = ua.to_ascii_lowercase();
    // 爬蟲、空 UA、範例清單、格式不符的 slug（char(10)）、無法解析的 IP（inet 欄位）都不寫入
    let Ok(addr) = ip.parse::<std::net::IpAddr>() else {
        return Ok(StatusCode::NO_CONTENT);
    };
    if ua.is_empty()
        || BOTS.iter().any(|b| lua.contains(b))
        || slug.starts_with("demo")
        || !crate::validate::slug_ok(&slug)
    {
        return Ok(StatusCode::NO_CONTENT);
    }
    // 超限仍回 204 但不寫入：每 IP 60/分，另對單一清單全域 600/分（擋分散來源灌水）。
    // 已被灌入的髒資料不會自動清除（紀錄永久保存），需人工查詢處理。
    if !crate::ratelimit::allow(&st.pool, &format!("share_view:{}", ip_key(addr)), 60, 60).await?
        || !crate::ratelimit::allow(&st.pool, &format!("share_view_slug:{slug}"), 600, 60).await?
    {
        return Ok(StatusCode::NO_CONTENT);
    }
    // 各欄位獨立解析：一個壞欄位不影響其他
    let b: serde_json::Value = serde_json::from_slice(&body).unwrap_or_default();
    let visitor_id = b["visitor_id"]
        .as_str()
        .and_then(|v| uuid::Uuid::parse_str(v).ok());
    let referrer = b["referrer_host"]
        .as_str()
        .map(str::to_ascii_lowercase)
        .filter(|h| host_ok(h));
    let (country, city) = geo(
        &parts.headers,
        crate::config::get().trusted_proxy_cloudflare,
    );
    let ua: String = ua.chars().take(512).collect();
    sqlx::query(
        "INSERT INTO share_views (wishlist_id, visitor_id, ip, country, city, device, user_agent, referrer_host)
         SELECT id, $2, $3::inet, $4, $5, $6, $7, $8 FROM wishlists WHERE slug = $1::char(10) AND deleted_at IS NULL",
    )
    .bind(&slug)
    .bind(visitor_id)
    .bind(&ip)
    .bind(country)
    .bind(city)
    .bind(device(&ua))
    .bind(&ua)
    .bind(referrer)
    .execute(&st.pool)
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(pairs: &[(&'static str, &'static str)]) -> HeaderMap {
        let mut m = HeaderMap::new();
        for (k, v) in pairs {
            m.insert(*k, v.parse().unwrap());
        }
        m
    }

    #[test]
    fn geo_trusted_reads_and_normalizes() {
        let m = h(&[("cf-ipcountry", "tw"), ("cf-ipcity", "Taipei")]);
        assert_eq!(geo(&m, true), (Some("TW".into()), Some("Taipei".into())));
    }

    #[test]
    fn geo_drops_invalid_country_and_untrusted() {
        assert_eq!(geo(&h(&[("cf-ipcountry", "XXX")]), true).0, None);
        assert_eq!(geo(&h(&[("cf-ipcountry", "1a")]), true).0, None);
        let m = h(&[("cf-ipcountry", "TW"), ("cf-ipcity", "Taipei")]);
        assert_eq!(geo(&m, false), (None, None));
    }

    #[test]
    fn device_classes() {
        assert_eq!(device("Mozilla/5.0 (iPad; CPU OS 17_0)"), "tablet");
        assert_eq!(device("Mozilla/5.0 (Linux; Android 14; SM-X900)"), "tablet");
        assert_eq!(device("Mozilla/5.0 (Linux; Android 14) Mobile"), "mobile");
        assert_eq!(device("Mozilla/5.0 (Windows NT 10.0)"), "desktop");
    }

    #[test]
    fn host_rules() {
        assert!(host_ok("l.facebook.com"));
        assert!(!host_ok("localhost"));
        assert!(!host_ok("1.2.3.4"));
        assert!(!host_ok("a b.com"));
    }

    #[test]
    fn ipv6_keyed_by_64() {
        let a: std::net::IpAddr = "2001:db8:1:2::1".parse().unwrap();
        let b: std::net::IpAddr = "2001:db8:1:2:ffff::9".parse().unwrap();
        assert_eq!(ip_key(a), ip_key(b));
        assert_eq!(ip_key("1.2.3.4".parse().unwrap()), "1.2.3.4");
        assert_eq!(ip_key("::ffff:1.2.3.4".parse().unwrap()), "1.2.3.4");
    }
}
