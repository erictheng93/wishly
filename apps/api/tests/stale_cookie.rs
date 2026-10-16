//! 失效的 guest token：header 帶的 → 401；只有 cookie 帶的（清不掉的殘留）→ 視為匿名，建立新訪客並覆蓋 cookie。
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
        .body(Body::from(
            json!({"qty": 1, "display_name": "新訪客"}).to_string(),
        ))
        .unwrap()
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn stale_cookie_is_anonymous_but_stale_header_is_401(pool: PgPool) {
    let a = app(&pool);
    let (_, its) = wishlist(&pool, "StaleTok01", 2, 10).await;
    // header 明確帶了失效 token → 401
    let (s, _, b) = call_with_ip(
        &a,
        claim_req(its[0], ("x-guest-token", "dead-token")),
        "2.2.2.2",
    )
    .await;
    assert_eq!(
        (s, b["code"].as_str()),
        (StatusCode::UNAUTHORIZED, Some("UNAUTHORIZED"))
    );
    // 只有失效的 ws_guest cookie → 視為匿名：建立新訪客、回新 token，並以 Set-Cookie 覆蓋舊的
    let (s, h, b) = call_with_ip(
        &a,
        claim_req(its[1], ("cookie", "ws_guest=dead-cookie")),
        "2.2.2.2",
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    assert!(b["guest_token"].as_str().is_some());
    assert!(h["set-cookie"].to_str().unwrap().starts_with("ws_guest="));
    common::assert_all_reconciled(&pool).await;
}
