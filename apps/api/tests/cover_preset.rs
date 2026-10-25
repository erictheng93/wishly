mod common;
use common::{call, ck};
use serde_json::json;
use sqlx::PgPool;

#[sqlx::test(migrations = "../../db/migrations")]
async fn cover_preset_create_patch_public(pool: PgPool) {
    let u = common::user(&pool, "u").await;
    let (s, _, b) = call(&pool, "POST", "/wishlists", &ck(&u.token), Some(json!({ "type": "personal", "title": "t", "cover_preset": "baby", "visibility": "public" }))).await;
    assert_eq!(s.as_u16(), 201, "{b}");
    assert_eq!(b["cover_preset"], "baby");
    let (id, slug) = (
        b["id"].as_str().unwrap().to_string(),
        b["slug"].as_str().unwrap().to_string(),
    );

    // 不帶欄位 = 不動；改值；null 清除
    let (_, _, b) = call(
        &pool,
        "PATCH",
        &format!("/wishlists/{id}"),
        &ck(&u.token),
        Some(json!({ "title": "t2" })),
    )
    .await;
    assert_eq!(b["cover_preset"], "baby", "{b}");
    let (s, _, b) = call(
        &pool,
        "PATCH",
        &format!("/wishlists/{id}"),
        &ck(&u.token),
        Some(json!({ "cover_preset": "travel" })),
    )
    .await;
    assert_eq!(
        (s.as_u16(), &b["cover_preset"]),
        (200, &json!("travel")),
        "{b}"
    );

    // 公開詳情 / 探索 / 個人頁（需上線且有品項）
    sqlx::query("UPDATE wishlists SET status='active' WHERE id=$1::uuid")
        .bind(&id)
        .execute(&pool)
        .await
        .unwrap();
    common::item(&pool, id.parse().unwrap(), "x", 1).await;
    let (s, _, b) = call(
        &pool,
        "GET",
        &format!("/public/wishlists/{slug}"),
        &[],
        None,
    )
    .await;
    assert_eq!(
        (s.as_u16(), &b["cover_preset"]),
        (200, &json!("travel")),
        "{b}"
    );
    let (_, _, b) = call(&pool, "GET", "/public/wishlists", &[], None).await;
    assert_eq!(b["wishlists"][0]["cover_preset"], "travel", "{b}");
    let (_, _, b) = call(&pool, "GET", "/wishlists", &ck(&u.token), None).await;
    assert_eq!(b["data"][0]["cover_preset"], "travel", "{b}");

    let (_, _, b) = call(
        &pool,
        "PATCH",
        &format!("/wishlists/{id}"),
        &ck(&u.token),
        Some(json!({ "cover_preset": null })),
    )
    .await;
    assert!(b["cover_preset"].is_null(), "{b}");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn cover_preset_rejects_unknown(pool: PgPool) {
    let u = common::user(&pool, "u").await;
    let (s, _, b) = call(
        &pool,
        "POST",
        "/wishlists",
        &ck(&u.token),
        Some(json!({ "type": "personal", "title": "t", "cover_preset": "nope" })),
    )
    .await;
    assert_eq!(s.as_u16(), 422, "{b}");
    let (_, _, b) = call(
        &pool,
        "POST",
        "/wishlists",
        &ck(&u.token),
        Some(json!({ "type": "personal", "title": "t" })),
    )
    .await;
    assert!(b["cover_preset"].is_null());
    let (s, _, b) = call(
        &pool,
        "PATCH",
        &format!("/wishlists/{}", b["id"].as_str().unwrap()),
        &ck(&u.token),
        Some(json!({ "cover_preset": "x" })),
    )
    .await;
    assert_eq!(s.as_u16(), 422, "{b}");
}
