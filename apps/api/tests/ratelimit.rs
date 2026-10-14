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
use wishsync_api::{claims, guest, ratelimit, reports, uploads, AppState};

fn app(pool: &PgPool) -> Router {
    Router::new()
        .nest(
            "/api/v1",
            claims::routes()
                .merge(guest::routes())
                .merge(reports::routes())
                .merge(uploads::routes()),
        )
        .with_state(AppState { pool: pool.clone() })
}

async fn call_with_ip(app: &Router, mut req: Request<Body>, ip: Option<&str>) -> common::Resp {
    if let Some(ip) = ip {
        req.extensions_mut()
            .insert(ConnectInfo::<std::net::SocketAddr>(
                format!("{ip}:1234").parse().unwrap(),
            ));
    }
    common::call_request(app, req).await
}

async fn claim(app: &Router, item: Uuid, email: &str, ip: &str) -> StatusCode {
    let r = Request::post(format!("/api/v1/items/{item}/claims"))
        .header("content-type", "application/json")
        .header("idempotency-key", Uuid::new_v4().to_string())
        .body(Body::from(
            json!({"qty": 1, "display_name": "x", "email": email}).to_string(),
        ))
        .unwrap();
    call_with_ip(app, r, Some(ip)).await.0
}

async fn mails(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM notifications WHERE kind = 'claim.confirmation'")
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn check_blocks_with_retry_after(pool: PgPool) {
    for _ in 0..2 {
        ratelimit::check(&pool, "k", 2, 3600).await.unwrap();
    }
    let e = ratelimit::check(&pool, "k", 2, 3600).await.unwrap_err();
    let res = axum::response::IntoResponse::into_response(e);
    assert_eq!(res.status(), 429);
    let ra: u64 = res.headers()["retry-after"]
        .to_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!((1..=3600).contains(&ra));
    ratelimit::check(&pool, "other", 2, 3600).await.unwrap(); // key 獨立
    common::assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn claim_mail_per_email_limit_still_creates_claim(pool: PgPool) {
    let a = app(&pool);
    let (_, items) = common::quantity_wishlist(&pool, "RlMail0001", 5, 5).await;
    for (n, it) in items.iter().enumerate() {
        assert_eq!(
            claim(&a, *it, "victim@example.com", &format!("10.0.0.{n}")).await,
            StatusCode::CREATED
        ); // 認領本身都成功
    }
    assert_eq!(mails(&pool).await, 3); // 同 email 每小時只寄 3 封
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM claims")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 5);
    common::assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn claim_mail_per_guest_daily_limit(pool: PgPool) {
    let a = app(&pool);
    let (_, items) = common::quantity_wishlist(&pool, "RlGuest001", 13, 5).await;
    let r = Request::post(format!("/api/v1/items/{}/claims", items[0]))
        .header("content-type", "application/json")
        .header("idempotency-key", Uuid::new_v4().to_string())
        .body(Body::from(
            json!({"qty": 1, "display_name": "x", "email": "g@example.com"}).to_string(),
        ))
        .unwrap();
    let (_, _, b) = call_with_ip(&a, r, Some("1.1.1.1")).await;
    let tok = b["guest_token"].as_str().unwrap().to_string();
    for it in &items[1..12] {
        let r = Request::post(format!("/api/v1/items/{it}/claims"))
            .header("content-type", "application/json")
            .header("idempotency-key", Uuid::new_v4().to_string())
            .header("x-guest-token", &tok)
            .body(Body::from(json!({"qty": 1}).to_string()))
            .unwrap();
        assert_eq!(
            call_with_ip(&a, r, Some("1.1.1.1")).await.0,
            StatusCode::CREATED
        );
    }
    // email 先到上限 3；以 guest 維度驗證：把 email 計數歸零再看 guest 計數
    sqlx::query("DELETE FROM rate_limits WHERE key LIKE 'claim_mail:%'")
        .execute(&pool)
        .await
        .unwrap();
    let g: Uuid = sqlx::query_scalar("SELECT id FROM guests")
        .fetch_one(&pool)
        .await
        .unwrap();
    let n: i32 = sqlx::query_scalar("SELECT count FROM rate_limits WHERE key = $1")
        .bind(format!("claim_guest:{g}"))
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 3); // 3 次（第 4 次起 email 先擋、短路不再計 guest）
    sqlx::query("UPDATE rate_limits SET count = 10 WHERE key = $1")
        .bind(format!("claim_guest:{g}"))
        .execute(&pool)
        .await
        .unwrap();
    let before = mails(&pool).await;
    let r = Request::post(format!("/api/v1/items/{}/claims", items[12]))
        .header("content-type", "application/json")
        .header("idempotency-key", Uuid::new_v4().to_string())
        .header("x-guest-token", &tok)
        .body(Body::from(json!({"qty": 1}).to_string()))
        .unwrap();
    assert_eq!(
        call_with_ip(&a, r, Some("1.1.1.1")).await.0,
        StatusCode::CREATED
    );
    assert_eq!(mails(&pool).await, before); // 超過每日 10 封：不寄
    common::assert_all_reconciled(&pool).await;
}

fn report(slug: &str) -> Request<Body> {
    Request::post(format!("/api/v1/public/wishlists/{slug}/reports"))
        .header("content-type", "application/json")
        .body(Body::from(json!({"reason": "scam"}).to_string()))
        .unwrap()
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn report_limits(pool: PgPool) {
    let a = app(&pool);
    for i in 0..11 {
        common::quantity_wishlist(&pool, &format!("RlRep{i:05}"), 0, 5).await;
    }
    // 同 IP 同清單 24h 1 筆
    assert_eq!(
        call_with_ip(&a, report("RlRep00000"), Some("2.2.2.2"))
            .await
            .0,
        StatusCode::CREATED
    );
    let (s, h, b) = call_with_ip(&a, report("RlRep00000"), Some("2.2.2.2")).await;
    assert_eq!(
        (s, b["code"].as_str().unwrap()),
        (StatusCode::TOO_MANY_REQUESTS, "RATE_LIMITED")
    );
    assert!(h.contains_key("retry-after"));
    assert_eq!(
        call_with_ip(&a, report("RlRep00000"), Some("3.3.3.3"))
            .await
            .0,
        StatusCode::CREATED
    ); // 另一 IP 不受影響
       // 每 IP 每小時 10 筆（已用 1 筆於 2.2.2.2 的 report_ip；再 9 筆不同清單到第 10 筆，第 11 筆被擋）
       // 被擋的嘗試也計數：此 IP 已 2 次，再 8 筆到 10，下一筆被擋
    for i in 1..9 {
        assert_eq!(
            call_with_ip(&a, report(&format!("RlRep{i:05}")), Some("2.2.2.2"))
                .await
                .0,
            StatusCode::CREATED,
            "{i}"
        );
    }
    assert_eq!(
        call_with_ip(&a, report("RlRep00009"), Some("2.2.2.2"))
            .await
            .0,
        StatusCode::TOO_MANY_REQUESTS
    );
    common::assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn report_known_reporter_duplicate_returns_same_id(pool: PgPool) {
    let a = app(&pool);
    let (wl, _) = common::quantity_wishlist(&pool, "RlRepDup01", 0, 5).await;
    let g: Uuid = sqlx::query_scalar(
        "INSERT INTO guests (guest_token_hash, display_name) VALUES ($1, 'g') RETURNING id",
    )
    .bind(wishsync_api::session::hash_token("tok"))
    .fetch_one(&pool)
    .await
    .unwrap();
    let mk = || {
        let mut r = report("RlRepDup01");
        r.headers_mut()
            .insert("x-guest-token", "tok".parse().unwrap());
        r
    };
    let (_, _, b1) = call_with_ip(&a, mk(), Some("4.4.4.4")).await;
    let (s, _, b2) = call_with_ip(&a, mk(), Some("4.4.4.4")).await;
    assert_eq!((s, &b1["id"]), (StatusCode::CREATED, &b2["id"]));
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM content_reports WHERE wishlist_id = $1 AND reporter_guest_id = $2",
    )
    .bind(wl)
    .bind(g)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(n, 1);
    common::assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn presign_limit_per_user(pool: PgPool) {
    let a = app(&pool);
    let common::TestUser { id: uid, token } = common::user(&pool, "u").await;
    // 預填：本小時已用 60 次 → 第 61 次 429（不必真的連 S3）
    sqlx::query("INSERT INTO rate_limits (key, window_start, count) VALUES ($1, to_timestamp(floor(extract(epoch FROM now()) / 3600) * 3600), 60)")
        .bind(format!("presign:{uid}")).execute(&pool).await.unwrap();
    let r = Request::post("/api/v1/uploads/presign")
        .header("content-type", "application/json")
        .header("cookie", format!("ws_session={token}"))
        .body(Body::from(
            json!({"purpose": "cover", "content_type": "image/png", "content_length": 1000})
                .to_string(),
        ))
        .unwrap();
    let (s, h, b) = call_with_ip(&a, r, None).await;
    assert_eq!(
        (s, b["code"].as_str().unwrap()),
        (StatusCode::TOO_MANY_REQUESTS, "RATE_LIMITED")
    );
    assert!(h.contains_key("retry-after"));
    common::assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn cleanup_removes_old_rows(pool: PgPool) {
    sqlx::query("INSERT INTO idempotency_keys (key, scope, request_hash, created_at) VALUES ($1, 's', 'h', now() - interval '25 hours'), ($2, 's', 'h', now())")
        .bind(Uuid::new_v4()).bind(Uuid::new_v4()).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO rate_limits (key, window_start, count) VALUES ('old', now() - interval '3 days', 1), ('new', now(), 1)").execute(&pool).await.unwrap();
    ratelimit::cleanup(&pool).await.unwrap();
    let (i, r): (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM idempotency_keys), (SELECT count(*) FROM rate_limits)",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!((i, r), (1, 1));
    common::assert_all_reconciled(&pool).await;
}
