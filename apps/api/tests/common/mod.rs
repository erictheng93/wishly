#![allow(dead_code)] // 每個整合測試 binary 只使用部分 fixture。

use axum::{
    body::Body,
    http::{HeaderMap, Request, StatusCode},
    Router,
};
use http_body_util::BodyExt;
use serde_json::Value;
use sqlx::PgPool;
use std::{future::Future, time::Duration};
use tower::ServiceExt;
use uuid::Uuid;

pub type Resp = (StatusCode, HeaderMap, Value);

pub async fn call(app: &Router, req: Request<Body>) -> Resp {
    let res = app.clone().oneshot(req).await.unwrap();
    let (status, headers) = (res.status(), res.headers().clone());
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).expect("API response must be JSON")
    };
    (status, headers, body)
}

pub fn request(
    method: &str,
    uri: &str,
    headers: &[(&str, String)],
    body: Option<Value>,
) -> Request<Body> {
    let mut req = Request::builder().method(method).uri(uri);
    for (key, value) in headers {
        req = req.header(*key, value);
    }
    match body {
        Some(body) => req
            .header("content-type", "application/json")
            .body(Body::from(body.to_string())),
        None => req.body(Body::empty()),
    }
    .unwrap()
}

pub async fn api(
    pool: &PgPool,
    method: &str,
    uri: &str,
    headers: &[(&str, String)],
    body: Option<Value>,
) -> Resp {
    call(
        &wishsync_api::app(wishsync_api::AppState { pool: pool.clone() }),
        request(method, &format!("/api/v1{uri}"), headers, body),
    )
    .await
}

pub fn ck(token: &str) -> Vec<(&'static str, String)> {
    vec![("cookie", format!("ws_session={token}"))]
}

pub fn gt(token: &str) -> Vec<(&'static str, String)> {
    vec![("x-guest-token", token.to_string())]
}

pub async fn session(pool: &PgPool, id: Uuid) -> String {
    let token = Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO sessions (user_id, token_hash, expires_at) VALUES ($1, $2, now() + interval '1 day')")
        .bind(id).bind(wishsync_api::session::hash_token(&token)).execute(pool).await.unwrap();
    token
}

pub async fn user(pool: &PgPool, name: &str) -> (Uuid, String) {
    user_with_staff(pool, name, false).await
}

pub async fn user_with_staff(pool: &PgPool, name: &str, staff: bool) -> (Uuid, String) {
    let id = sqlx::query_scalar(
        "INSERT INTO users (display_name, email, is_staff) VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(name)
    .bind(format!("{}@example.com", Uuid::new_v4()))
    .bind(staff)
    .fetch_one(pool)
    .await
    .unwrap();
    (id, session(pool, id).await)
}

pub async fn wishlist(pool: &PgPool, owner: Uuid, slug: &str) -> Uuid {
    sqlx::query_scalar("INSERT INTO wishlists (owner_id, type, status, slug, title) VALUES ($1, 'registry', 'active', $2, '清單') RETURNING id")
        .bind(owner).bind(slug).fetch_one(pool).await.unwrap()
}

pub async fn assert_claims_reconciled(pool: &PgPool) {
    let drift: Vec<(Uuid, i32, i64)> = sqlx::query_as(
        "SELECT i.id, i.qty_claimed, COALESCE(sum(c.qty), 0)::bigint FROM wishlist_items i
         LEFT JOIN claims c ON c.item_id = i.id AND c.status IN ('reserved', 'purchased', 'delivered')
         GROUP BY i.id HAVING i.qty_claimed <> COALESCE(sum(c.qty), 0)")
        .fetch_all(pool).await.unwrap();
    assert!(
        drift.is_empty(),
        "qty_claimed 與有效 claims 加總不一致: {drift:?}"
    );
}

/// 上限只用於回報缺少事件；成功取決於 predicate，沒有固定 sleep。
pub async fn wait_until<F, Fut>(label: &str, mut ready: F)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = bool>,
{
    tokio::time::timeout(Duration::from_secs(10), async {
        while !ready().await {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {label}"));
}

pub async fn wait_for_blocked(pool: &PgPool, blocker_pid: i32, minimum: i64) {
    wait_until("DB operations blocked by fixture transaction", || async {
        let n: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_stat_activity WHERE datname = current_database() AND $1 = ANY(pg_blocking_pids(pid))")
            .bind(blocker_pid).fetch_one(pool).await.unwrap();
        n >= minimum
    }).await;
}

pub async fn claimed_wishlist(
    pool: &PgPool,
    owner: Uuid,
    surprise: bool,
    event_offset_days: i32,
) -> (Uuid, String, Uuid) {
    let slug: String = Uuid::new_v4().simple().to_string()[..10].to_string();
    let wid: Uuid = sqlx::query_scalar(
        "INSERT INTO wishlists (owner_id, type, status, slug, title, surprise_mode, event_date)
         VALUES ($1, 'registry', 'active', $2, '寶寶清單', $3, current_date + $4::int) RETURNING id")
        .bind(owner).bind(&slug).bind(surprise).bind(event_offset_days).fetch_one(pool).await.unwrap();
    let item: Uuid = sqlx::query_scalar("INSERT INTO wishlist_items (wishlist_id, title, qty_needed, qty_claimed) VALUES ($1, '玻璃奶瓶', 2, 1) RETURNING id")
        .bind(wid).fetch_one(pool).await.unwrap();
    let g: Uuid = sqlx::query_scalar("INSERT INTO guests (guest_token_hash, display_name, contact) VALUES ($1, '秘密客', 'line:secret') RETURNING id")
        .bind(wishsync_api::session::hash_token(&Uuid::new_v4().to_string())).fetch_one(pool).await.unwrap();
    sqlx::query(
        "INSERT INTO claims (item_id, guest_id, claimer_name, qty) VALUES ($1, $2, '秘密客', 1)",
    )
    .bind(item)
    .bind(g)
    .execute(pool)
    .await
    .unwrap();
    (wid, slug, item)
}

pub async fn guest_wishlist(pool: &PgPool, owner: Uuid, surprise: bool, off: i32) -> (Uuid, Uuid) {
    let slug: String = Uuid::new_v4().simple().to_string()[..10].to_string();
    let wid: Uuid = sqlx::query_scalar(
        "INSERT INTO wishlists (owner_id, type, status, slug, title, surprise_mode, event_date)
        VALUES ($1, 'registry', 'active', $2, '清單', $3, current_date + $4::int) RETURNING id",
    )
    .bind(owner)
    .bind(slug)
    .bind(surprise)
    .bind(off)
    .fetch_one(pool)
    .await
    .unwrap();
    let it: Uuid = sqlx::query_scalar("INSERT INTO wishlist_items (wishlist_id, title, qty_needed) VALUES ($1, '奶瓶', 3) RETURNING id").bind(wid).fetch_one(pool).await.unwrap();
    (wid, it)
}
pub async fn guest_claim(pool: &PgPool, item: Uuid, email: Option<&str>) -> (String, Uuid) {
    let (s, _, b) = api(
        pool,
        "POST",
        &format!("/items/{item}/claims"),
        &[("idempotency-key", Uuid::new_v4().to_string())],
        Some(serde_json::json!({ "qty": 1, "display_name": "小明", "email": email })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    (
        b["guest_token"].as_str().unwrap().into(),
        b["claim"]["id"].as_str().unwrap().parse().unwrap(),
    )
}

pub async fn quantity_wishlist(
    pool: &PgPool,
    slug: &str,
    items: usize,
    needed: i32,
) -> (Uuid, Vec<Uuid>) {
    let owner = sqlx::query_scalar("INSERT INTO users (display_name) VALUES ('o') RETURNING id")
        .fetch_one(pool)
        .await
        .unwrap();
    let list = wishlist(pool, owner, slug).await;
    let mut ids = Vec::new();
    for n in 0..items {
        ids.push(sqlx::query_scalar("INSERT INTO wishlist_items (wishlist_id, title, qty_needed) VALUES ($1, $2, $3) RETURNING id")
            .bind(list).bind(format!("i{n}")).bind(needed).fetch_one(pool).await.unwrap());
    }
    (list, ids)
}
