//! 部署探針：/healthz 不依賴 DB，/readyz 依賴 DB 且不洩漏錯誤內容。
mod common;
use axum::http::StatusCode;
use common::{call_request, request, test_app};
use sqlx::PgPool;

fn get(path: &str) -> axum::http::Request<axum::body::Body> {
    request("GET", path, &[], None)
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn healthz_ok_and_not_cacheable(pool: PgPool) {
    let (st, h, body) = call_request(&test_app(&pool), get("/healthz")).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(body["status"], "ok");
    assert_eq!(h["cache-control"], "no-store");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn readyz_ok_with_db(pool: PgPool) {
    let (st, _, body) = call_request(&test_app(&pool), get("/readyz")).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(body["status"], "ok");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn readyz_503_without_leaking_db_error_while_healthz_stays_ok(pool: PgPool) {
    let app = test_app(&pool);
    pool.close().await;
    let (st, _, body) = call_request(&app, get("/readyz")).await;
    assert_eq!(st, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body, serde_json::json!({ "status": "unavailable" }));
    let (st, _, _) = call_request(&app, get("/healthz")).await;
    assert_eq!(st, StatusCode::OK);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn probes_unaffected_by_read_only_and_not_under_api_prefix(pool: PgPool) {
    sqlx::query("INSERT INTO system_flags (key, value) VALUES ('read_only','true') ON CONFLICT (key) DO UPDATE SET value='true'")
        .execute(&pool).await.unwrap();
    let app = test_app(&pool);
    for p in ["/healthz", "/readyz"] {
        let (st, _, _) = call_request(&app, get(p)).await;
        assert_eq!(st, StatusCode::OK, "{p}");
    }
    let (st, _, _) = call_request(&app, get("/api/v1/healthz")).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
}
