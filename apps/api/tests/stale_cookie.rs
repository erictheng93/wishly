//! 失效的 guest token：不論 header 或 cookie 都不能認捐（401、不建 guest）；但不得擋住已登入會員。
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

fn claim_req(item: Uuid, extra: (&str, &str)) -> Request<Body> {
    Request::post(format!("/api/v1/items/{item}/claims"))
        .header("content-type", "application/json")
        .header("idempotency-key", Uuid::new_v4().to_string())
        .header(extra.0, extra.1)
        .body(Body::from(json!({"qty": 1}).to_string()))
        .unwrap()
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn stale_guest_token_never_creates_guest_and_never_blocks_user(pool: PgPool) {
    let a = app(&pool);
    let (_, its) = wishlist(&pool, "StaleTok01", 3, 10).await;
    // header / cookie 帶失效 token 且無 session → 401，且不建立任何東西
    for extra in [
        ("x-guest-token", "dead-token"),
        ("cookie", "ws_guest=dead-cookie"),
    ] {
        let (s, _, b) = call_with_ip(&a, claim_req(its[0], extra), "2.2.2.2").await;
        assert_eq!(
            (s, b["code"].as_str()),
            (StatusCode::UNAUTHORIZED, Some("UNAUTHORIZED"))
        );
    }
    let (g, c): (i64, i64) =
        sqlx::query_as("SELECT (SELECT count(*) FROM guests), (SELECT count(*) FROM claims)")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!((g, c), (0, 0));
    // 已登入會員殘留失效的 ws_guest cookie：照常認捐，且不發 Set-Cookie
    let u = common::user(&pool, "會員").await;
    let cookie = format!("ws_session={}; ws_guest=dead-cookie", u.token);
    let (s, h, b) = call_with_ip(&a, claim_req(its[1], ("cookie", &cookie)), "2.2.2.2").await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    assert!(!h.contains_key("set-cookie"));
    common::assert_all_reconciled(&pool).await;
}
