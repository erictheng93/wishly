//! 支持品項：不計入完成度與急需統計（本任務尚無 API，以 SQL 直接插入）。
mod common;
use axum::http::StatusCode;
use common::*;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

async fn support_item(pool: &PgPool, wl: Uuid, target: Uuid, relief: bool) {
    sqlx::query(
        "INSERT INTO wishlist_items(wishlist_id,title,qty_needed,funding_mode,support_wishlist_id,category,urgency)
         VALUES($1,'支持',2,'support',$2,$3,$4)",
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

#[sqlx::test(migrations = "../../db/migrations")]
async fn explore_keeps_list_with_only_support_items(pool: PgPool) {
    let owner = user(&pool, "owner").await;
    let other = user(&pool, "other").await;
    let target = wishlist(&pool, other.id, ListOpts::default()).await;
    let list = wishlist(&pool, owner.id, ListOpts::default()).await;
    support_item(&pool, list.id, target.id, false).await;
    let (s, _, v) = call(&pool, "GET", "/public/wishlists", &[], None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let l = v["wishlists"]
        .as_array()
        .unwrap()
        .iter()
        .find(|w| w["slug"] == list.slug.as_str())
        .expect("留在探索頁");
    assert_eq!(l["item_count"], 1);
    assert_eq!(l["completion_pct"], 0);
}

async fn create(pool: &PgPool, tok: &str, wl: Uuid, slug: &str) -> (StatusCode, Value) {
    let (s, _, v) = call(
        pool,
        "POST",
        &format!("/wishlists/{wl}/items"),
        &ck(tok),
        Some(json!({ "title": "支持", "funding_mode": "support", "support_slug": slug })),
    )
    .await;
    (s, v)
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn create_support_item_without_date(pool: PgPool) {
    let a = user(&pool, "a").await;
    let b = user(&pool, "b").await;
    let target = wishlist(&pool, b.id, ListOpts::default()).await;
    let list = wishlist(&pool, a.id, ListOpts::default()).await;
    let (s, v) = create(&pool, &a.token, list.id, &target.slug).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    assert_eq!(v["funding_mode"], "support");
    assert!(v["funding_deadline"].is_null());
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn create_rejects_unavailable_target(pool: PgPool) {
    let a = user(&pool, "a").await;
    let b = user(&pool, "b").await;
    let list = wishlist(&pool, a.id, ListOpts::default()).await;
    let friends = wishlist(
        &pool,
        b.id,
        ListOpts {
            visibility: Some("friends"),
            ..Default::default()
        },
    )
    .await;
    let private = wishlist(
        &pool,
        b.id,
        ListOpts {
            visibility: Some("private"),
            ..Default::default()
        },
    )
    .await;
    let mut details = vec![];
    for slug in [friends.slug.as_str(), private.slug.as_str(), "nonexistent"] {
        let (s, v) = create(&pool, &a.token, list.id, slug).await;
        assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
        assert_eq!(v["code"], "SUPPORT_TARGET_INVALID");
        details.push(v["detail"].clone());
    }
    assert!(details.windows(2).all(|w| w[0] == w[1]));
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn create_rejects_own_list(pool: PgPool) {
    let a = user(&pool, "a").await;
    let own = wishlist(&pool, a.id, ListOpts::default()).await;
    let list = wishlist(&pool, a.id, ListOpts::default()).await;
    let (s, v) = create(&pool, &a.token, list.id, &own.slug).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    assert_eq!(v["code"], "SUPPORT_TARGET_SELF");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn create_rejects_manageable_org_list(pool: PgPool) {
    let owner = user(&pool, "o").await;
    let coord = user(&pool, "c").await;
    let o = org(&pool, owner.id, "verified").await;
    member(&pool, o, owner.id, "owner").await;
    member(&pool, o, coord.id, "coordinator").await;
    let p = permit(&pool, o, "approved", 30).await;
    let (e, _) = event(&pool, o).await;
    let st = site(&pool, o, owner.id, e, p, "active").await;
    let list = wishlist(&pool, coord.id, ListOpts::default()).await;
    let (s, v) = create(&pool, &coord.token, list.id, &st.slug).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    assert_eq!(v["code"], "SUPPORT_TARGET_SELF");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn create_rate_limited(pool: PgPool) {
    let a = user(&pool, "a").await;
    let b = user(&pool, "b").await;
    let target = wishlist(&pool, b.id, ListOpts::default()).await;
    let list = wishlist(&pool, a.id, ListOpts::default()).await;
    for _ in 0..20 {
        let (s, v) = create(&pool, &a.token, list.id, &target.slug).await;
        assert_eq!(s, StatusCode::CREATED, "{v}");
    }
    let (s, _) = create(&pool, &a.token, list.id, &target.slug).await;
    assert_eq!(s, StatusCode::TOO_MANY_REQUESTS);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn support_mode_cannot_change(pool: PgPool) {
    let a = user(&pool, "a").await;
    let b = user(&pool, "b").await;
    let target = wishlist(&pool, b.id, ListOpts::default()).await;
    let list = wishlist(&pool, a.id, ListOpts::default()).await;
    let (_, v) = create(&pool, &a.token, list.id, &target.slug).await;
    let id = v["id"].as_str().unwrap();
    let (s, _, v) = call(
        &pool,
        "PATCH",
        &format!("/items/{id}"),
        &ck(&a.token),
        Some(json!({ "funding_mode": "crowdfund" })),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["code"], "FUNDING_MODE_LOCKED");
    let q = item(&pool, list.id, "一般", 1).await;
    let (s, _, v) = call(
        &pool,
        "PATCH",
        &format!("/items/{q}"),
        &ck(&a.token),
        Some(json!({ "funding_mode": "support" })),
    )
    .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn support_target_is_immutable(pool: PgPool) {
    let a = user(&pool, "a").await;
    let b = user(&pool, "b").await;
    let target = wishlist(&pool, b.id, ListOpts::default()).await;
    let list = wishlist(&pool, a.id, ListOpts::default()).await;
    let (_, v) = create(&pool, &a.token, list.id, &target.slug).await;
    let id = v["id"].as_str().unwrap();
    let (s, _, v) = call(
        &pool,
        "PATCH",
        &format!("/items/{id}"),
        &ck(&a.token),
        Some(json!({ "support_slug": target.slug })),
    )
    .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn claim_and_pledge_rejected_on_support_item(pool: PgPool) {
    let a = user(&pool, "a").await;
    let b = user(&pool, "b").await;
    let target = wishlist(&pool, b.id, ListOpts::default()).await;
    let list = wishlist(&pool, a.id, ListOpts::default()).await;
    let (_, v) = create(&pool, &a.token, list.id, &target.slug).await;
    let id = v["id"].as_str().unwrap();
    let d = user(&pool, "d").await;
    for (path, body) in [
        ("claims", json!({ "qty": 1, "visibility": "public" })),
        ("contributions", json!({ "points": 10 })),
    ] {
        let h = [ck(&d.token), idem(Uuid::new_v4())].concat();
        let (s, _, v) = call(
            &pool,
            "POST",
            &format!("/items/{id}/{path}"),
            &h,
            Some(body),
        )
        .await;
        assert!(!s.is_success(), "{path} {v}");
    }
    let (c, p): (i32, i64) =
        sqlx::query_as("SELECT qty_claimed, pledged_points FROM wishlist_items WHERE id=$1::uuid")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!((c, p), (0, 0));
}
