//! POST /public/wishlists/{slug}/views：分享頁瀏覽紀錄（未登入訪客）。
//! 公開清單 GET 會被 CDN 快取、不可讀 cookie，所以由前端掛載後另外回報。
//! 一律回 204（不洩漏清單是否存在）；IP 只寫入 share_views，不進任何回應或日誌。
use crate::{error::AppError, ratelimit::Peer, AppState};
use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{header, request::Parts, StatusCode},
    routing::post,
    Router,
};
use serde::Deserialize;

pub fn routes() -> Router<AppState> {
    Router::new().route("/public/wishlists/{slug}/views", post(record))
}

#[derive(Deserialize, Default)]
struct Body {
    visitor_id: Option<String>,
    referrer_host: Option<String>,
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

fn host_ok(h: &str) -> bool {
    !h.is_empty()
        && h.len() <= 253
        && h.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
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
    // 爬蟲、空 UA、範例清單、無法解析的 IP（inet 欄位）都不寫入
    if ua.is_empty()
        || BOTS.iter().any(|b| lua.contains(b))
        || slug.starts_with("demo")
        || !crate::validate::slug_ok(&slug)
        || ip.parse::<std::net::IpAddr>().is_err()
    {
        return Ok(StatusCode::NO_CONTENT);
    }
    // 超限仍回 204 但不寫入
    if !crate::ratelimit::allow(&st.pool, &format!("share_view:{ip}"), 60, 60).await? {
        return Ok(StatusCode::NO_CONTENT);
    }
    let b: Body = serde_json::from_slice(&body).unwrap_or_default();
    let visitor_id = b.visitor_id.and_then(|v| uuid::Uuid::parse_str(&v).ok());
    let referrer = b
        .referrer_host
        .map(|h| h.to_ascii_lowercase())
        .filter(|h| host_ok(h));
    // 國家 / 城市只在信任 Cloudflare 時讀取，否則任何人都能偽造
    let (country, city) = if crate::config::get().trusted_proxy_cloudflare {
        let h = |k: &str| {
            parts
                .headers
                .get(k)
                .and_then(|v| v.to_str().ok())
                .map(|v| v.chars().take(100).collect::<String>())
                .filter(|v| !v.is_empty())
        };
        (
            h("cf-ipcountry")
                .filter(|c| c.len() == 2 && c.bytes().all(|b| b.is_ascii_alphabetic())),
            h("cf-ipcity"),
        )
    } else {
        (None, None)
    };
    let ua: String = ua.chars().take(512).collect();
    sqlx::query(
        "INSERT INTO share_views (wishlist_id, visitor_id, ip, country, city, device, user_agent, referrer_host)
         SELECT id, $2, $3::inet, $4, $5, $6, $7, $8 FROM wishlists WHERE slug = $1 AND deleted_at IS NULL",
    )
    .bind(&slug)
    .bind(visitor_id)
    .bind(&ip)
    .bind(country.map(|c| c.to_ascii_uppercase()))
    .bind(city)
    .bind(device(&ua))
    .bind(&ua)
    .bind(referrer)
    .execute(&st.pool)
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
