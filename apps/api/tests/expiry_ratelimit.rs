//! 捐助逾期釋放（FR-08）與捐助端點限流。
mod common;
use axum::{
    body::Body,
    extract::ConnectInfo,
    http::{Request, StatusCode},
    Router,
};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;
use wishly_api::{claims, guest, AppState};

fn app(pool: &PgPool) -> Router {
    Router::new()
        .nest("/api/v1", claims::routes().merge(guest::routes()))
        .with_state(AppState { pool: pool.clone() })
}

async fn call_with_ip(app: &Router, mut req: Request<Body>, ip: &str) -> common::Resp {
    req.extensions_mut()
        .insert(ConnectInfo::<std::net::SocketAddr>(
            format!("{ip}:1234").parse().unwrap(),
        ));
    common::call_request(app, req).await
}

use common::quantity_wishlist as wishlist;

/// 直接塞捐助並同步 qty_claimed；回傳 claim id
async fn seed_claim(pool: &PgPool, item: Uuid, qty: i32, status: &str, expires: &str) -> Uuid {
    let g: Uuid = sqlx::query_scalar(
        "INSERT INTO guests (guest_token_hash, display_name) VALUES ($1, 'g') RETURNING id",
    )
    .bind(Uuid::new_v4().as_bytes().to_vec())
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query("UPDATE wishlist_items SET qty_claimed = qty_claimed + $2 WHERE id = $1")
        .bind(item)
        .bind(qty)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query_scalar(sqlx::AssertSqlSafe(format!("INSERT INTO claims (item_id, guest_id, claimer_name, qty, status, expires_at) VALUES ($1, $2, 'g', $3, $4::claim_status, {expires}) RETURNING id")))
        .bind(item).bind(g).bind(qty).bind(status).fetch_one(pool).await.unwrap()
}

async fn qty(pool: &PgPool, item: Uuid) -> i32 {
    sqlx::query_scalar("SELECT qty_claimed FROM wishlist_items WHERE id = $1")
        .bind(item)
        .fetch_one(pool)
        .await
        .unwrap()
}
async fn status(pool: &PgPool, c: Uuid) -> String {
    sqlx::query_scalar("SELECT status::text FROM claims WHERE id = $1")
        .bind(c)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn expire_due_releases_only_overdue_reserved(pool: PgPool) {
    let (_, items) = wishlist(&pool, "ExpDue0001", 2, 10).await;
    let (a, b) = (items[0], items[1]);
    let due1 = seed_claim(&pool, a, 2, "reserved", "now() - interval '1 minute'").await;
    let due2 = seed_claim(&pool, a, 3, "reserved", "now() - interval '1 hour'").await;
    let live = seed_claim(&pool, a, 1, "reserved", "now() + interval '1 hour'").await;
    let no_ttl = seed_claim(&pool, a, 1, "reserved", "NULL").await;
    let bought = seed_claim(&pool, b, 4, "purchased", "now() - interval '1 day'").await; // 已購買：不動
    assert_eq!(qty(&pool, a).await, 7);
    assert_eq!(claims::expire_due(&pool).await.unwrap(), 2);
    assert_eq!(qty(&pool, a).await, 2); // 7 - 2 - 3
    assert_eq!(qty(&pool, b).await, 4);
    for (c, s) in [
        (due1, "expired"),
        (due2, "expired"),
        (live, "reserved"),
        (no_ttl, "reserved"),
        (bought, "purchased"),
    ] {
        assert_eq!(status(&pool, c).await, s);
    }
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_logs WHERE action = 'claim.expire' AND actor_type = 'system'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(n, 2);
    // 冪等：再跑不重複回補
    assert_eq!(claims::expire_due(&pool).await.unwrap(), 0);
    assert_eq!(qty(&pool, a).await, 2);
    common::assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn expire_due_concurrent_and_vs_cancel(pool: PgPool) {
    let (_, items) = wishlist(&pool, "ExpRace001", 1, 10).await;
    let it = items[0];
    let c1 = seed_claim(&pool, it, 2, "reserved", "now() - interval '1 minute'").await;
    seed_claim(&pool, it, 3, "reserved", "now() - interval '1 minute'").await;
    // 多實例同時跑：總共只回補一次
    let rs = futures_util::future::join_all((0..4).map(|_| claims::expire_due(&pool))).await;
    assert_eq!(rs.into_iter().map(|r| r.unwrap()).sum::<usize>(), 2);
    assert_eq!(qty(&pool, it).await, 0);
    // 已 expired 的捐助再被取消：狀態轉換被拒，不會二次回補
    let g: Uuid = sqlx::query_scalar("SELECT guest_id FROM claims WHERE id = $1")
        .bind(c1)
        .fetch_one(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE guests SET guest_token_hash = $2 WHERE id = $1")
        .bind(g)
        .bind(sha(b"tok"))
        .execute(&pool)
        .await
        .unwrap();
    let r = Request::delete(format!("/api/v1/claims/{c1}"))
        .header("x-guest-token", "tok")
        .body(Body::empty())
        .unwrap();
    let (s, _, b) = call_with_ip(&app(&pool), r, "9.9.9.9").await;
    assert_eq!(
        (s, b["code"].as_str()),
        (StatusCode::CONFLICT, Some("INVALID_STATE_TRANSITION"))
    );
    assert_eq!(qty(&pool, it).await, 0);
    common::assert_all_reconciled(&pool).await;
}

fn sha(b: &[u8]) -> Vec<u8> {
    use sha2::{Digest, Sha256};
    Sha256::digest(b).to_vec()
}

fn post_claim_as(item: Uuid, key: Uuid, token: &str) -> Request<Body> {
    Request::post(format!("/api/v1/items/{item}/claims"))
        .header("content-type", "application/json")
        .header("idempotency-key", key.to_string())
        .header("cookie", format!("ws_session={token}"))
        .body(Body::from(json!({"qty": 1}).to_string()))
        .unwrap()
}

async fn new_user(pool: &PgPool) -> String {
    common::user(pool, "x").await.token
}

fn limited(r: &common::Resp) -> bool {
    r.0 == StatusCode::TOO_MANY_REQUESTS
        && r.2["code"] == "RATE_LIMITED"
        && r.1.contains_key("retry-after")
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn claim_limit_is_per_identity_and_replay_free(pool: PgPool) {
    let a = app(&pool);
    // 5 份清單各 7 個品項：避開「同清單 15 次」
    let mut its = vec![];
    for w in 0..5 {
        its.extend(wishlist(&pool, &format!("RlId{w:06}"), 7, 100).await.1);
    }
    let tok = new_user(&pool).await;
    // 同 key 重播：只消耗一次額度
    let k = Uuid::new_v4();
    assert_eq!(
        call_with_ip(&a, post_claim_as(its[1], k, &tok), "5.5.5.5")
            .await
            .0,
        StatusCode::CREATED
    );
    for _ in 0..40 {
        assert_eq!(
            call_with_ip(&a, post_claim_as(its[1], k, &tok), "5.5.5.5")
                .await
                .0,
            StatusCode::CREATED
        );
    }
    // 已計 1 次（its[1]），再 29 次成功，下一次 429
    for it in &its[2..31] {
        assert_eq!(
            call_with_ip(&a, post_claim_as(*it, Uuid::new_v4(), &tok), "5.5.5.5")
                .await
                .0,
            StatusCode::CREATED
        );
    }
    assert!(limited(
        &call_with_ip(&a, post_claim_as(its[31], Uuid::new_v4(), &tok), "5.5.5.5").await
    ));
    // 同一個 IP 的另一個人不受影響（派對 Wi-Fi / CGNAT）
    let tok2 = new_user(&pool).await;
    assert_eq!(
        call_with_ip(&a, post_claim_as(its[33], Uuid::new_v4(), &tok2), "5.5.5.5")
            .await
            .0,
        StatusCode::CREATED
    );
    // 超限後重播先前成功的請求仍可回放
    assert_eq!(
        call_with_ip(&a, post_claim_as(its[1], k, &tok), "5.5.5.5")
            .await
            .0,
        StatusCode::CREATED
    );
    common::assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn claim_per_wishlist_limit_is_per_identity(pool: PgPool) {
    let a = app(&pool);
    let (_, its) = wishlist(&pool, "RlWl000001", 20, 100).await;
    let tok = new_user(&pool).await;
    // 同清單 15 次成功，第 16 次 429
    for it in &its[..15] {
        assert_eq!(
            call_with_ip(&a, post_claim_as(*it, Uuid::new_v4(), &tok), "7.7.7.7")
                .await
                .0,
            StatusCode::CREATED
        );
    }
    assert!(limited(
        &call_with_ip(&a, post_claim_as(its[15], Uuid::new_v4(), &tok), "7.7.7.7").await
    ));
    // 同 IP、同清單的其他人照常捐助（場地 Wi-Fi 的賓客）
    let other = new_user(&pool).await;
    for it in &its[16..20] {
        assert_eq!(
            call_with_ip(&a, post_claim_as(*it, Uuid::new_v4(), &other), "7.7.7.7")
                .await
                .0,
            StatusCode::CREATED
        );
    }
    common::assert_all_reconciled(&pool).await;
}
