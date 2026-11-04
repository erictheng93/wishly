//! ORGS_ENABLED 關閉時，檢舉組織路由回 404（config 為全域單例，所以獨立成一個 binary）。
mod common;
use axum::http::StatusCode;
use common::{call, user};
use serde_json::json;
use sqlx::PgPool;

#[sqlx::test(migrations = "../../db/migrations")]
async fn org_report_route_is_404_when_orgs_disabled(pool: PgPool) {
    wishly_api::config::init(wishly_api::config::Config {
        orgs_enabled: false,
        ..wishly_api::config::Config::dev()
    });
    let owner = user(&pool, "owner").await;
    let o = common::org(&pool, owner.id, "verified").await;
    let slug: String = sqlx::query_scalar("SELECT slug::text FROM organizations WHERE id=$1")
        .bind(o)
        .fetch_one(&pool)
        .await
        .unwrap();
    let (s, ..) = call(
        &pool,
        "POST",
        &format!("/public/orgs/{slug}/reports"),
        &[],
        Some(json!({"reason":"scam"})),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM content_reports")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 0);
}
