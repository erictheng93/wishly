//! POST /pwa-events（匿名、白名單、限流）與 GET /admin/analytics 的 pwa 區塊。
mod common;
use common::*;
use serde_json::json;
use sqlx::PgPool;

fn ev(event: &str, platform: &str, context: &str) -> serde_json::Value {
    json!({ "event": event, "platform": platform, "context": context })
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn accepts_valid_and_rejects_unknown_values(pool: PgPool) {
    let (st, _, _) = call(&pool, "POST", "/pwa-events", &[], Some(ev("pwa_install_prompted", "ios", "claim"))).await;
    assert_eq!(st, 204);
    for bad in [
        ev("hack", "ios", "claim"),
        ev("pwa_installed", "windows", "claim"),
        ev("pwa_installed", "ios", "x"),
    ] {
        let (st, _, b) = call(&pool, "POST", "/pwa-events", &[], Some(bad)).await;
        assert_eq!(st, 422, "{b}");
    }
    let (n,): (i64,) = sqlx::query_as("SELECT count(*) FROM pwa_events").fetch_one(&pool).await.unwrap();
    assert_eq!(n, 1);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn rate_limited_per_ip(pool: PgPool) {
    for _ in 0..30 {
        let (st, _, _) = call(&pool, "POST", "/pwa-events", &[], Some(ev("pwa_installed", "android", "appinstalled"))).await;
        assert_eq!(st, 204);
    }
    let (st, _, _) = call(&pool, "POST", "/pwa-events", &[], Some(ev("pwa_installed", "android", "appinstalled"))).await;
    assert_eq!(st, 429);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn admin_analytics_counts_pwa_in_range(pool: PgPool) {
    let s = staff(&pool, "營運").await;
    for e in [ev("pwa_install_prompted", "ios", "claim"), ev("pwa_install_prompted", "android", "fund"), ev("pwa_installed", "android", "appinstalled")] {
        call(&pool, "POST", "/pwa-events", &[], Some(e)).await;
    }
    // 區間外的舊資料不計
    sqlx::query("INSERT INTO pwa_events (event, platform, context, created_at) VALUES ('pwa_installed', 'ios', 'first_run', now() - interval '60 days')")
        .execute(&pool).await.unwrap();
    let (st, _, b) = call(&pool, "GET", "/admin/analytics?days=7", &ck(&s.token), None).await;
    assert_eq!(st, 200, "{b}");
    assert_eq!(b["pwa"], json!({ "prompted": 2, "installed": 1 }));
    let (_, _, b) = call(&pool, "GET", "/admin/analytics?days=90", &ck(&s.token), None).await;
    assert_eq!(b["pwa"]["installed"], 2);
}
