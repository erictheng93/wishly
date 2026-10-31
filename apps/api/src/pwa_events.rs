//! PWA 安裝分析事件：匿名 client 事件接收端。
//! 不放 /public/*（那裡會被 CDN 快取）；只收白名單值，不存使用者、IP、UA。
use crate::{error::AppError, ratelimit::Peer, AppState};
use axum::{extract::State, http::StatusCode, routing::post, Json, Router};
use serde::Deserialize;

pub fn routes() -> Router<AppState> {
    Router::new().route("/pwa-events", post(record))
}

#[derive(Deserialize)]
struct Body {
    event: String,
    platform: String,
    context: String,
}

async fn record(
    State(st): State<AppState>,
    Peer(ip): Peer,
    Json(b): Json<Body>,
) -> Result<StatusCode, AppError> {
    let bad = |f: &str| AppError::invalid(f, "INVALID_VALUE", "不在允許的值內");
    if !["pwa_install_prompted", "pwa_installed"].contains(&b.event.as_str()) {
        return Err(bad("/event"));
    }
    if !["ios", "android"].contains(&b.platform.as_str()) {
        return Err(bad("/platform"));
    }
    if !["claim", "fund", "appinstalled", "first_run"].contains(&b.context.as_str()) {
        return Err(bad("/context"));
    }
    crate::ratelimit::check(&st.pool, &format!("pwa_event:{ip}"), 30, 3600).await?;
    sqlx::query("INSERT INTO pwa_events (event, platform, context) VALUES ($1, $2, $3)")
        .bind(&b.event)
        .bind(&b.platform)
        .bind(&b.context)
        .execute(&st.pool)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
