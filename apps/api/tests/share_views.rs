//! POST /public/wishlists/{slug}/views：分享頁瀏覽紀錄與營運分析。
mod common;
use axum::{body::Body, extract::ConnectInfo, http::Request};
use common::*;
use serde_json::{json, Value};
use sqlx::PgPool;
use std::net::SocketAddr;

const UA: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) Mobile/15E148";

async fn view(
    pool: &PgPool,
    slug: &str,
    ip: &str,
    ua: Option<&str>,
    extra: &[(&str, String)],
    body: Value,
) -> u16 {
    let mut h: Vec<(&str, String)> = extra.to_vec();
    if let Some(ua) = ua {
        h.push(("user-agent", ua.to_string()));
    }
    let mut req: Request<Body> = request(
        "POST",
        &format!("/api/v1/public/wishlists/{slug}/views"),
        &h,
        Some(body),
    );
    req.extensions_mut().insert(ConnectInfo(
        format!("{ip}:1234").parse::<SocketAddr>().unwrap(),
    ));
    call_request(&test_app(pool), req).await.0.as_u16()
}

async fn rows(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM share_views")
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn list(pool: &PgPool) -> TestList {
    let o = user(pool, "A").await;
    wishlist(pool, o.id, ListOpts::default()).await
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn writes_one_row(pool: PgPool) {
    let w = list(&pool).await;
    let vid = uuid::Uuid::new_v4();
    let st = view(
        &pool,
        &w.slug,
        "1.2.3.4",
        Some(UA),
        &[("cf-ipcountry", "TW".into())],
        json!({"visitor_id": vid, "referrer_host": "Example.com"}),
    )
    .await;
    assert_eq!(st, 204);
    let r: (uuid::Uuid, String, String, String, Option<String>, Option<uuid::Uuid>, Option<String>) = sqlx::query_as(
        "SELECT wishlist_id, host(ip), device, user_agent, referrer_host, visitor_id, country FROM share_views",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(r.0, w.id);
    assert_eq!(r.1, "1.2.3.4");
    assert_eq!(r.2, "mobile");
    assert_eq!(r.3, UA);
    assert_eq!(r.4.as_deref(), Some("example.com"));
    assert_eq!(r.5, Some(vid));
    // 未設 TRUSTED_PROXY=cloudflare：CF 標頭不可信，一律忽略
    assert_eq!(r.6, None);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn skipped_cases_still_204(pool: PgPool) {
    let w = list(&pool).await;
    // 不存在的 slug
    assert_eq!(
        view(&pool, "abcdefghij", "1.1.1.1", Some(UA), &[], json!({})).await,
        204
    );
    // 已刪除的清單
    sqlx::query("UPDATE wishlists SET deleted_at = now() WHERE id = $1")
        .bind(w.id)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        view(&pool, &w.slug, "1.1.1.1", Some(UA), &[], json!({})).await,
        204
    );
    let w2 = list(&pool).await;
    for ua in [
        Some("Googlebot/2.1"),
        Some("facebookexternalhit/1.1"),
        Some("WhatsApp/2"),
        Some(""),
        None,
    ] {
        assert_eq!(
            view(&pool, &w2.slug, "1.1.1.1", ua, &[], json!({})).await,
            204
        );
    }
    // 範例清單
    assert_eq!(
        view(&pool, "demo", "1.1.1.1", Some(UA), &[], json!({})).await,
        204
    );
    assert_eq!(
        view(&pool, "demo123456", "1.1.1.1", Some(UA), &[], json!({})).await,
        204
    );
    assert_eq!(rows(&pool).await, 0);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn malformed_fields_become_null(pool: PgPool) {
    let w = list(&pool).await;
    view(
        &pool,
        &w.slug,
        "1.1.1.1",
        Some(UA),
        &[],
        json!({"visitor_id": "nope", "referrer_host": "a b/c"}),
    )
    .await;
    let r: (Option<uuid::Uuid>, Option<String>) =
        sqlx::query_as("SELECT visitor_id, referrer_host FROM share_views")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(r, (None, None));
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn rate_limited_still_204_without_row(pool: PgPool) {
    let w = list(&pool).await;
    for _ in 0..62 {
        assert_eq!(
            view(&pool, &w.slug, "5.5.5.5", Some(UA), &[], json!({})).await,
            204
        );
    }
    assert_eq!(rows(&pool).await, 60);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn analytics_counts_and_privacy(pool: PgPool) {
    let s = staff(&pool, "營運").await;
    let u = user(&pool, "一般").await;
    let w = list(&pool).await;
    let vid = uuid::Uuid::new_v4();
    // 同一 visitor 3 次（不同 IP）+ 4 個無 visitor_id 的不同 IP = 6 位訪客、8 次瀏覽（改為 5 個無 visitor_id）
    for ip in ["1.0.0.1", "1.0.0.2", "1.0.0.3"] {
        view(
            &pool,
            &w.slug,
            ip,
            Some(UA),
            &[],
            json!({"visitor_id": vid, "referrer_host": "l.facebook.com"}),
        )
        .await;
    }
    for ip in ["2.0.0.1", "2.0.0.2", "2.0.0.3", "2.0.0.4", "2.0.0.5"] {
        view(
            &pool,
            &w.slug,
            ip,
            Some("Mozilla/5.0 (Windows NT 10.0)"),
            &[],
            json!({}),
        )
        .await;
    }
    let (st, _, _) = call(&pool, "GET", "/admin/analytics", &ck(&u.token), None).await;
    assert_eq!(st, 403);
    let (st, _, b) = call(&pool, "GET", "/admin/analytics?days=7", &ck(&s.token), None).await;
    assert_eq!(st, 200, "{b}");
    assert_eq!(b["visits"], json!({"views": 8, "visitors": 6}));
    assert_eq!(b["totals"]["visitors"], 6);
    assert_eq!(b["daily"][6]["views"], 8);
    assert_eq!(b["daily"][6]["visitors"], 6);
    assert_eq!(b["daily"][0]["views"], 0);
    assert_eq!(b["distribution"]["view_device"]["mobile"], Value::Null);
    assert_eq!(b["distribution"]["view_device"]["desktop"], 5);
    // 少於 5 抑制
    assert_eq!(
        b["distribution"]["view_referrer"]["l.facebook.com"],
        Value::Null
    );
    assert_eq!(b["distribution"]["view_referrer"]["direct"], 5);
    assert!(b["distribution"]["view_country"].is_object());
    // 任何回應都不含 IP
    let text = b.to_string();
    assert!(
        !text.contains("1.0.0.") && !text.contains("2.0.0."),
        "{text}"
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn empty_distribution_keys_present(pool: PgPool) {
    let s = staff(&pool, "營運").await;
    let (_, _, b) = call(&pool, "GET", "/admin/analytics", &ck(&s.token), None).await;
    for k in ["view_device", "view_country", "view_referrer"] {
        assert!(b["distribution"][k].as_object().unwrap().is_empty(), "{k}");
    }
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn cleanup_keeps_share_views_forever(pool: PgPool) {
    let w = list(&pool).await;
    view(&pool, &w.slug, "1.1.1.1", Some(UA), &[], json!({})).await;
    sqlx::query("UPDATE share_views SET created_at = now() - interval '10 years'")
        .execute(&pool)
        .await
        .unwrap();
    wishly_api::ratelimit::cleanup(&pool).await.unwrap();
    assert_eq!(rows(&pool).await, 1);
}
