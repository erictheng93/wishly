//! 外部連結：整組覆寫、URL 驗證、公開連結頁不依 cookie。
mod common;
use axum::http::StatusCode;
use common::{call, ck, user};
use serde_json::{json, Value};
use sqlx::PgPool;

async fn put(pool: &PgPool, token: &str, links: Value) -> (StatusCode, Value) {
    let (s, _, b) = call(
        pool,
        "PUT",
        "/me/links",
        &ck(token),
        Some(json!({ "links": links })),
    )
    .await;
    (s, b)
}
async fn setup(pool: &PgPool) -> common::TestUser {
    let a = user(pool, "A").await;
    sqlx::query("UPDATE users SET handle='alice' WHERE id=$1")
        .bind(a.id)
        .execute(pool)
        .await
        .unwrap();
    a
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn put_replaces_and_orders(pool: PgPool) {
    let a = setup(&pool).await;
    let (s, _) = put(
        &pool,
        &a.token,
        json!([{"title":"一","url":"https://a.com"},{"title":"二","url":"https://b.com"}]),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (s, b) = put(&pool, &a.token, json!([{"title":"B","url":"https://b.com"},{"title":"A","url":"https://a.com","hidden":true}])).await;
    assert_eq!(s, StatusCode::OK);
    let t: Vec<&str> = b["links"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["title"].as_str().unwrap())
        .collect();
    assert_eq!(t, ["B", "A"]);
    let (_, _, g) = call(&pool, "GET", "/me/links", &ck(&a.token), None).await;
    assert_eq!(g["links"].as_array().unwrap().len(), 2);
    assert_eq!(g["links"][1]["hidden"], true);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn rejects_bad_urls(pool: PgPool) {
    let a = setup(&pool).await;
    for u in ["http://a.com", "https://x:y@a.com", "javascript:alert(1)"] {
        let (s, b) = put(
            &pool,
            &a.token,
            json!([{"title":"x","url":"https://ok.com"},{"title":"x","url":u}]),
        )
        .await;
        assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{u}");
        assert_eq!(b["errors"][0]["pointer"], "/links/1/url");
    }
    let many: Vec<Value> = (0..11)
        .map(|i| json!({"title":"x","url":format!("https://a.com/{i}")}))
        .collect();
    assert_eq!(
        put(&pool, &a.token, json!(many)).await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn public_links_hides_hidden(pool: PgPool) {
    let a = setup(&pool).await;
    put(&pool, &a.token, json!([{"title":"show","url":"https://a.com"},{"title":"hid","url":"https://b.com","hidden":true}])).await;
    let (s, _, b) = call(&pool, "GET", "/public/links/ALICE", &[], None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(b["links"].as_array().unwrap().len(), 1);
    assert_eq!(b["links"][0]["title"], "show");
    assert_eq!(b["user"]["handle"], "alice");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn public_links_ignores_cookie(pool: PgPool) {
    let a = setup(&pool).await;
    put(
        &pool,
        &a.token,
        json!([{"title":"hid","url":"https://b.com","hidden":true}]),
    )
    .await;
    let (s, h, b) = call(&pool, "GET", "/public/links/alice", &ck(&a.token), None).await;
    assert_eq!(s, StatusCode::OK);
    assert!(b["links"].as_array().unwrap().is_empty());
    assert!(h.get("set-cookie").is_none());
    assert_eq!(h.get("cache-control").unwrap(), "public, s-maxage=60");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn public_links_404_for_suspended(pool: PgPool) {
    let a = setup(&pool).await;
    sqlx::query("UPDATE users SET blocked_at=now() WHERE id=$1")
        .bind(a.id)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        call(&pool, "GET", "/public/links/alice", &[], None).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(&pool, "GET", "/public/links/nobody", &[], None)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn profile_includes_links(pool: PgPool) {
    let a = setup(&pool).await;
    put(&pool, &a.token, json!([{"title":"show","url":"https://a.com"},{"title":"hid","url":"https://b.com","hidden":true}])).await;
    let (s, _, b) = call(&pool, "GET", "/users/alice", &[], None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(
        b["user"]["links"],
        json!([{"title":"show","url":"https://a.com"}])
    );
}
