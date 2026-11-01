//! 支持品項：不計入完成度與急需統計（本任務尚無 API，以 SQL 直接插入）。
mod common;
use axum::http::StatusCode;
use common::*;
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

async fn support_item(pool: &PgPool, wl: Uuid, target: Uuid, relief: bool) {
    sqlx::query(
        "INSERT INTO wishlist_items(wishlist_id,title,qty_needed,funding_mode,support_wishlist_id,category,urgency)
         VALUES($1,'支持',1,'support',$2,$3,$4)",
    )
    .bind(wl)
    .bind(target)
    .bind(relief.then_some("food"))
    .bind(relief.then_some("critical"))
    .execute(pool)
    .await
    .unwrap();
}

async fn completion(pool: &PgPool, slug: &str) -> Value {
    let (s, _, v) = call(pool, "GET", &format!("/public/wishlists/{slug}"), &[], None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v["completion"].clone()
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn support_item_excluded_from_completion(pool: PgPool) {
    let owner = user(&pool, "owner").await;
    let other = user(&pool, "other").await;
    let target = wishlist(&pool, other.id, ListOpts::default()).await;
    let list = wishlist(&pool, owner.id, ListOpts::default()).await;
    item(&pool, list.id, "一般", 2).await;
    support_item(&pool, list.id, target.id, false).await;
    let c = completion(&pool, &list.slug).await;
    assert_eq!(c["item_count"], 1);
    assert_eq!(c["fulfilled_count"], 0);
    assert_eq!(c["completion_pct"], 0);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn only_support_items_is_not_complete(pool: PgPool) {
    let owner = user(&pool, "owner").await;
    let other = user(&pool, "other").await;
    let target = wishlist(&pool, other.id, ListOpts::default()).await;
    let list = wishlist(&pool, owner.id, ListOpts::default()).await;
    support_item(&pool, list.id, target.id, false).await;
    let c = completion(&pool, &list.slug).await;
    assert_eq!(c["item_count"], 0);
    assert_eq!(c["fulfilled_count"], 0);
    assert_eq!(c["completion_pct"], 0);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn relief_needs_ignore_support_item(pool: PgPool) {
    let owner = user(&pool, "O").await;
    let o = org(&pool, owner.id, "verified").await;
    member(&pool, o, owner.id, "owner").await;
    let p = permit(&pool, o, "approved", 30).await;
    let (e, slug) = event(&pool, o).await;
    let site = site(&pool, o, owner.id, e, p, "active").await;
    let target = wishlist(&pool, owner.id, ListOpts::default()).await;
    support_item(&pool, site.id, target.id, true).await;
    let (s, _, v) = call(
        &pool,
        "GET",
        &format!("/public/relief-events/{slug}"),
        &[],
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["sites"][0]["needs"]["critical"], 0, "{v}");
}
