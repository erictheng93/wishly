//! 共用測試 helper；此目錄不會被 Cargo 當成獨立測試 binary。
#![allow(dead_code)] // 每個整合測試 binary 只使用部分 fixture。

pub mod cdn;

use axum::{
    body::Body,
    http::{HeaderMap, Request, StatusCode},
    Router,
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use sqlx::PgPool;
use std::{future::Future, time::Duration};
use tower::ServiceExt;
use uuid::Uuid;

pub type Resp = (StatusCode, HeaderMap, Value);
pub type Hdr = Vec<(&'static str, String)>;

pub fn test_app(pool: &PgPool) -> Router {
    wishly_api::app(wishly_api::AppState { pool: pool.clone() })
}

/// URI 不含 /api/v1；可傳入隔離的 Router 或併發共用的 Router。
pub async fn call_app(
    app: &Router,
    method: &str,
    uri: &str,
    hdr: &[(&str, String)],
    body: Option<Value>,
) -> Resp {
    call_request(app, request(method, &format!("/api/v1{uri}"), hdr, body)).await
}

pub struct TestUser {
    pub id: Uuid,
    pub token: String,
}
pub async fn user(pool: &PgPool, name: &str) -> TestUser {
    create_user(pool, name, false).await
}
pub async fn staff(pool: &PgPool, name: &str) -> TestUser {
    create_user(pool, name, true).await
}
pub async fn befriend(pool: &PgPool, a: Uuid, b: Uuid) {
    let (a, b) = if a < b { (a, b) } else { (b, a) };
    sqlx::query("INSERT INTO friendships(user_a,user_b) VALUES($1,$2)")
        .bind(a)
        .bind(b)
        .execute(pool)
        .await
        .unwrap();
}

/// 未指定時採 active / public / ok。
#[derive(Default)]
pub struct ListOpts {
    pub status: Option<&'static str>,
    pub visibility: Option<&'static str>,
    pub surprise: bool,
    pub event_offset_days: Option<i32>,
    pub closes_in: Option<&'static str>,
    pub moderation_status: Option<&'static str>,
}
pub struct TestList {
    pub id: Uuid,
    pub slug: String,
}
pub async fn wishlist(pool: &PgPool, owner: Uuid, o: ListOpts) -> TestList {
    let slug = Uuid::new_v4().simple().to_string()[..10].to_string();
    let id = sqlx::query_scalar("INSERT INTO wishlists(owner_id,type,status,slug,title,visibility,surprise_mode,event_date,closes_at,moderation_status,moderation_reason)
        VALUES($1,'registry',$2::wishlist_status,$3,'清單',$4::visibility,$5,current_date+$6::int,now()+$7::interval,$8::moderation_status,
        CASE WHEN $8='hidden' THEN 'test fixture' END) RETURNING id")
        .bind(owner).bind(o.status.unwrap_or("active")).bind(&slug).bind(o.visibility.unwrap_or("public"))
        .bind(o.surprise).bind(o.event_offset_days).bind(o.closes_in).bind(o.moderation_status.unwrap_or("ok"))
        .fetch_one(pool).await.unwrap();
    TestList { id, slug }
}
pub async fn item(pool: &PgPool, wl: Uuid, title: &str, needed: i32) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO wishlist_items(wishlist_id,title,qty_needed) VALUES($1,$2,$3) RETURNING id",
    )
    .bind(wl)
    .bind(title)
    .bind(needed)
    .fetch_one(pool)
    .await
    .unwrap()
}
pub async fn crowdfund_item(pool: &PgPool, wl: Uuid, target_points: i64) -> Uuid {
    sqlx::query_scalar("INSERT INTO wishlist_items(wishlist_id,title,qty_needed,funding_mode,target_points,funding_status,funding_deadline,fulfillment_type)
        VALUES($1,'嬰兒推車',1,'crowdfund',$2,'open',now()+interval '7 days','concierge') RETURNING id")
        .bind(wl).bind(target_points).fetch_one(pool).await.unwrap()
}
pub fn idem(key: Uuid) -> Hdr {
    vec![("idempotency-key", key.to_string())]
}
pub async fn grant(pool: &PgPool, user: Uuid, n: i64) {
    let mut tx = pool.begin().await.unwrap();
    let wallet = wishly_api::points::ensure_wallet(&mut tx, user)
        .await
        .unwrap();
    wishly_api::points::post(
        &mut tx,
        wallet,
        n,
        wishly_api::points::Entry {
            kind: "grant",
            ref_type: "manual",
            ref_id: None,
            note: Some("test"),
            actor: None,
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
}
pub async fn balance(pool: &PgPool, user: Uuid) -> i64 {
    sqlx::query_scalar("SELECT COALESCE((SELECT balance FROM point_wallets WHERE user_id=$1),0)")
        .bind(user)
        .fetch_one(pool)
        .await
        .unwrap()
}
pub async fn pledge(app: &Router, item: Uuid, token: &str, points: i64) -> Resp {
    call_app(
        app,
        "POST",
        &format!("/items/{item}/contributions"),
        &[ck(token), idem(Uuid::new_v4())].concat(),
        Some(json!({"points":points})),
    )
    .await
}
/// 以 SQL 建一筆已入帳的 topups 列，再走 `points::post`（kind=topup）入帳——與 webhook 相同的入帳路徑；回傳 topups.id。
pub async fn topup_credit(pool: &PgPool, user: Uuid, n: i64) -> Uuid {
    let mut tx = pool.begin().await.unwrap();
    let wallet = wishly_api::points::ensure_wallet(&mut tx, user)
        .await
        .unwrap();
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO topups (id, wallet_id, merchant_trade_no, provider, method, amount, status, provider_txn_id, paid_at, expires_at)
                 VALUES ($1, $2, $3, 'ecpay', 'card', $4, 'paid', $3, now(), now())")
        .bind(id).bind(wallet).bind(id.to_string()).bind(n).execute(&mut *tx).await.unwrap();
    wishly_api::points::post(
        &mut tx,
        wallet,
        n,
        wishly_api::points::Entry {
            kind: "topup",
            ref_type: "topup",
            ref_id: Some(id),
            note: None,
            actor: None,
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    id
}
/// 直接植入 gift_card / reward_bonus 批次（目前無對應功能）：批次 + balance + ledger 一次補齊，維持對帳。
pub async fn seed_lot(
    pool: &PgPool,
    user: Uuid,
    source: &str,
    n: i64,
    expires_in: Option<&str>,
) -> Uuid {
    let mut tx = pool.begin().await.unwrap();
    let wallet = wishly_api::points::ensure_wallet(&mut tx, user)
        .await
        .unwrap();
    let lot: Uuid = sqlx::query_scalar("INSERT INTO point_lots (wallet_id, source, source_ref_id, original_points, remaining_points, cash_refundable, expires_at)
                 VALUES ($1, $2::point_lot_source, gen_random_uuid(), $3, $3, false, now() + $4::interval) RETURNING id")
        .bind(wallet).bind(source).bind(n).bind(expires_in).fetch_one(&mut *tx).await.unwrap();
    let after: i64 = sqlx::query_scalar(
        "UPDATE point_wallets SET balance = balance + $2 WHERE id = $1 RETURNING balance",
    )
    .bind(wallet)
    .bind(n)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    sqlx::query("INSERT INTO point_ledger (wallet_id, lot_id, delta, balance_after, entry_type, ref_type) VALUES ($1, $2, $3, $4, 'grant', 'manual')")
        .bind(wallet).bind(lot).bind(n).bind(after).execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    lot
}
/// 各批次 (source, remaining) 依建立順序（測試用快照）
pub async fn lots(pool: &PgPool, user: Uuid) -> Vec<(String, i64)> {
    sqlx::query_as("SELECT l.source::text, l.remaining_points FROM point_lots l JOIN point_wallets w ON w.id = l.wallet_id WHERE w.user_id = $1 ORDER BY l.created_at, l.id")
        .bind(user).fetch_all(pool).await.unwrap()
}
pub async fn assert_points_reconciled(pool: &PgPool) {
    let bad: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM point_wallets w WHERE balance <> COALESCE((SELECT sum(delta) FROM point_ledger l WHERE l.wallet_id=w.id),0)")
        .fetch_all(pool).await.unwrap();
    assert!(bad.is_empty(), "錢包 balance 與 ledger 加總不一致: {bad:?}");
    let drift: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM wishlist_items i WHERE funding_mode='crowdfund' AND pledged_points <> COALESCE((SELECT sum(points) FROM contributions c WHERE c.item_id=i.id AND c.status IN ('pledged','captured')),0)")
        .fetch_all(pool).await.unwrap();
    assert!(drift.is_empty(), "item.pledged_points 漂移: {drift:?}");
    // 批次：Σ lots.remaining = balance；批次 remaining = 該批次 ledger 加總（舊資料批次的 ledger 沒有 lot_id，只對帳錢包層級）
    let lot_bad: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM point_wallets w WHERE balance <> COALESCE((SELECT sum(remaining_points) FROM point_lots l WHERE l.wallet_id=w.id),0)")
        .fetch_all(pool).await.unwrap();
    assert!(
        lot_bad.is_empty(),
        "錢包 balance 與批次 remaining 加總不一致: {lot_bad:?}"
    );
    let lot_drift: Vec<Uuid> = sqlx::query_scalar("SELECT l.id FROM point_lots l WHERE NOT (l.source = 'adjustment' AND l.source_ref_id = l.wallet_id) AND l.remaining_points <> COALESCE((SELECT sum(delta) FROM point_ledger g WHERE g.lot_id = l.id),0)")
        .fetch_all(pool).await.unwrap();
    assert!(
        lot_drift.is_empty(),
        "批次 remaining 與 ledger 加總不一致: {lot_drift:?}"
    );
    let alloc_drift: Vec<Uuid> = sqlx::query_scalar("SELECT c.id FROM contributions c WHERE c.status IN ('pledged','captured') AND EXISTS (SELECT 1 FROM contribution_lot_allocations a WHERE a.contribution_id = c.id) AND c.points - c.refunded_points <> (SELECT sum(a.points - a.refunded_points) FROM contribution_lot_allocations a WHERE a.contribution_id = c.id)")
        .fetch_all(pool).await.unwrap();
    assert!(
        alloc_drift.is_empty(),
        "認捐與批次分配不一致: {alloc_drift:?}"
    );
}
pub async fn assert_all_reconciled(pool: &PgPool) {
    assert_points_reconciled(pool).await;
    assert_claims_reconciled(pool).await;
}

/// 原始 Request 的 URI 含 /api/v1；供需要注入 ConnectInfo 的測試使用。
pub async fn call_request(app: &Router, req: Request<Body>) -> Resp {
    let res = app.clone().oneshot(req).await.unwrap();
    let (status, headers) = (res.status(), res.headers().clone());
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let body = if bytes.is_empty() {
        Value::Null
    } else if headers
        .get("content-type")
        .is_some_and(|h| h.to_str().is_ok_and(|s| s.contains("json")))
    {
        serde_json::from_slice(&bytes).expect("JSON response must be valid")
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
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

/// 完整 app，URI 不含 /api/v1。
pub async fn call(
    pool: &PgPool,
    method: &str,
    uri: &str,
    headers: &[(&str, String)],
    body: Option<Value>,
) -> Resp {
    call_app(&test_app(pool), method, uri, headers, body).await
}

pub fn ck(token: &str) -> Hdr {
    vec![("cookie", format!("ws_session={token}"))]
}

pub fn gt(token: &str) -> Hdr {
    vec![("x-guest-token", token.to_string())]
}

/// 最新一張 OTP 挑戰的 id（匿名端點發出的挑戰驗證時必須帶 challenge_id）。
pub async fn latest_challenge(pool: &PgPool, email: &str, purpose: &str) -> String {
    sqlx::query_scalar::<_, Uuid>("SELECT id FROM otp_challenges WHERE email=$1 AND purpose=$2 ORDER BY created_at DESC LIMIT 1")
        .bind(email)
        .bind(purpose)
        .fetch_one(pool)
        .await
        .unwrap()
        .to_string()
}

pub async fn session(pool: &PgPool, id: Uuid) -> String {
    let token = Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO sessions (user_id, token_hash, expires_at) VALUES ($1, $2, now() + interval '1 day')")
        .bind(id).bind(wishly_api::session::hash_token(&token)).execute(pool).await.unwrap();
    token
}

async fn create_user(pool: &PgPool, name: &str, staff: bool) -> TestUser {
    let id = sqlx::query_scalar(
        "INSERT INTO users (display_name, email, is_staff) VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(name)
    .bind(format!("{}@example.com", Uuid::new_v4()))
    .bind(staff)
    .fetch_one(pool)
    .await
    .unwrap();
    TestUser {
        id,
        token: session(pool, id).await,
    }
}

pub async fn assert_claims_reconciled(pool: &PgPool) {
    let drift: Vec<(Uuid, i32, i64)> = sqlx::query_as(
        "SELECT i.id, i.qty_claimed, COALESCE(sum(c.qty), 0)::bigint FROM wishlist_items i
         LEFT JOIN claims c ON c.item_id = i.id AND c.status IN ('reserved', 'purchased', 'delivered')
         GROUP BY i.id HAVING i.qty_claimed <> COALESCE(sum(c.qty), 0) OR i.qty_claimed < 0 OR i.qty_claimed > i.qty_needed")
        .fetch_all(pool).await.unwrap();
    assert!(
        drift.is_empty(),
        "qty_claimed 與有效 claims 加總不一致: {drift:?}"
    );
}

/// 成功取決於 predicate；5ms 間隔只降低輪詢負載，逾時上限只診斷缺少事件。
pub async fn wait_until<F, Fut>(label: &str, mut ready: F)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = bool>,
{
    tokio::time::timeout(Duration::from_secs(10), async {
        while !ready().await {
            tokio::time::sleep(Duration::from_millis(5)).await;
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

/// 隱私測試的秘密訪客 fixture；數量均由呼叫端明確指定，無 API 通知副作用。
pub async fn claimed_wishlist(
    pool: &PgPool,
    owner: Uuid,
    surprise: bool,
    event_offset_days: i32,
    needed: i32,
    qty: i32,
) -> (Uuid, String, Uuid) {
    let list = wishlist(
        pool,
        owner,
        ListOpts {
            surprise,
            event_offset_days: Some(event_offset_days),
            ..Default::default()
        },
    )
    .await;
    let wid = list.id;
    sqlx::query("UPDATE wishlists SET title='寶寶清單' WHERE id=$1")
        .bind(wid)
        .execute(pool)
        .await
        .unwrap();
    let item = item(pool, wid, "玻璃奶瓶", needed).await;
    sqlx::query("UPDATE wishlist_items SET qty_claimed=$2 WHERE id=$1")
        .bind(item)
        .bind(qty)
        .execute(pool)
        .await
        .unwrap();
    let g: Uuid = sqlx::query_scalar("INSERT INTO guests (guest_token_hash, display_name, contact) VALUES ($1, '秘密客', 'line:secret') RETURNING id")
        .bind(wishly_api::session::hash_token(&Uuid::new_v4().to_string())).fetch_one(pool).await.unwrap();
    sqlx::query(
        "INSERT INTO claims (item_id, guest_id, claimer_name, qty) VALUES ($1, $2, '秘密客', $3)",
    )
    .bind(item)
    .bind(g)
    .bind(qty)
    .execute(pool)
    .await
    .unwrap();
    (wid, list.slug, item)
}

/// 舊資料訪客捐助（新認捐已需登入）：直接以 SQL 建 guest + reserved claim，並維持 qty_claimed 一致。
pub async fn legacy_guest_claim(
    pool: &PgPool,
    item: Uuid,
    qty: i32,
    email: Option<&str>,
) -> (String, Uuid) {
    let token = Uuid::new_v4().to_string();
    let g: Uuid = sqlx::query_scalar("INSERT INTO guests (guest_token_hash, display_name, email) VALUES ($1, '小明', $2) RETURNING id")
        .bind(wishly_api::session::hash_token(&token)).bind(email).fetch_one(pool).await.unwrap();
    let claim = sqlx::query_scalar("INSERT INTO claims (item_id, guest_id, claimer_name, qty) VALUES ($1, $2, '小明', $3) RETURNING id")
        .bind(item).bind(g).bind(qty).fetch_one(pool).await.unwrap();
    sqlx::query("UPDATE wishlist_items SET qty_claimed = qty_claimed + $2 WHERE id = $1")
        .bind(item)
        .bind(qty)
        .execute(pool)
        .await
        .unwrap();
    (token, claim)
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
    // 這個 fixture 刻意無 email，避免限流案例建立額外通知。
    let list = wishlist(pool, owner, ListOpts::default()).await.id;
    sqlx::query("UPDATE wishlists SET slug=$2 WHERE id=$1")
        .bind(list)
        .bind(slug)
        .execute(pool)
        .await
        .unwrap();
    let mut ids = Vec::new();
    for n in 0..items {
        ids.push(item(pool, list, &format!("i{n}"), needed).await);
    }
    (list, ids)
}

/// 建立新會員並透過 API 捐助；回傳 (session token, claim id)。
pub async fn user_claim(pool: &PgPool, item: Uuid, qty: i32) -> (String, Uuid) {
    let u = user(pool, "小明").await;
    let (s, _, b) = call(
        pool,
        "POST",
        &format!("/items/{item}/claims"),
        &[ck(&u.token), idem(Uuid::new_v4())].concat(),
        Some(json!({ "qty": qty })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    (u.token, b["claim"]["id"].as_str().unwrap().parse().unwrap())
}
