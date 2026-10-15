//! 部署探針（掛在根路徑，不屬 /api/v1 公開契約）。
//! /healthz：liveness，不碰 DB，避免 DB 短暫故障讓編排層（Dokploy／Docker）不斷重啟服務。
//! /readyz：readiness／外部監控，`SELECT 1` 加 2 秒逾時；失敗只回固定訊息，不洩漏錯誤內容。
use crate::AppState;
use axum::{extract::State, http::StatusCode, routing::get, Json, Router};
use serde_json::{json, Value};
use std::time::Duration;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
}

async fn healthz() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}

async fn readyz(State(st): State<AppState>) -> (StatusCode, Json<Value>) {
    let probe = sqlx::query_scalar::<_, i32>("SELECT 1").fetch_one(&st.pool);
    match tokio::time::timeout(Duration::from_secs(2), probe).await {
        Ok(Ok(_)) => (StatusCode::OK, Json(json!({ "status": "ok" }))),
        Ok(Err(e)) => {
            tracing::warn!(error = %e, "readyz: db query failed");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "status": "unavailable" })),
            )
        }
        Err(_) => {
            tracing::warn!("readyz: db query timed out");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "status": "unavailable" })),
            )
        }
    }
}
