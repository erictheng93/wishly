mod common;
use axum::http::StatusCode;
use sqlx::PgPool;
use uuid::Uuid;

/// 回傳 (user_id, session token)
use common::*;

#[sqlx::test(migrations = "../../db/migrations")]
async fn surprise_dashboard_hides_claimers(pool: PgPool) {
    let TestUser {
        id: owner,
        token: tok,
    } = user(&pool, "owner").await;
    let TestUser {
        id: stranger,
        token: stok,
    } = user(&pool, "x").await;
    let _ = stranger;
    let (wid, _, _) = claimed_wishlist(&pool, owner, true, 30).await;
    let (s, _, v) = call(
        &pool,
        "GET",
        &format!("/wishlists/{wid}/dashboard"),
        &ck(&tok),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let raw = v.to_string();
    assert!(
        !raw.contains("秘密客") && !raw.contains("line:secret"),
        "leak: {raw}"
    );
    assert_eq!(v["surprise_locked"], true);
    assert!(v["claims"].is_null() && v["items"][0]["qty_claimed"].is_null());
    assert_eq!(v["totals"]["completion_pct"], 50); // 數量計 1/2（totals.qty_claimed 本就輸出）
                                                   // 非擁有者 → 404
    assert_eq!(
        call(
            &pool,
            "GET",
            &format!("/wishlists/{wid}/dashboard"),
            &ck(&stok),
            None
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(
            &pool,
            "GET",
            &format!("/wishlists/{wid}/dashboard"),
            &[],
            None
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    // 活動日已過 → 解鎖，看得到認領者
    sqlx::query("UPDATE wishlists SET event_date = current_date - 1 WHERE id = $1")
        .bind(wid)
        .execute(&pool)
        .await
        .unwrap();
    let (_, _, v) = call(
        &pool,
        "GET",
        &format!("/wishlists/{wid}/dashboard"),
        &ck(&tok),
        None,
    )
    .await;
    assert_eq!(v["surprise_locked"], false);
    assert_eq!(v["claims"].as_array().map(|a| a.len()), Some(1));
    assert_eq!(v["items"][0]["qty_claimed"], 1);
    assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn unified_pct_owner_names_list_fields_and_image_base(pool: PgPool) {
    let TestUser {
        id: owner,
        token: tok,
    } = user(&pool, "owner").await;
    let (wid, slug, _) = claimed_wishlist(&pool, owner, false, -1).await; // 1/2 claimed, show_claimer_names 預設 false
                                                                          // 第二品項 1/3（數量計 2/5=40；以品項計會是 0 或 50）；soft-deleted 品項不計
    let second: Uuid = sqlx::query_scalar("INSERT INTO wishlist_items (wishlist_id, title, qty_needed, qty_claimed) VALUES ($1, 'b', 3, 1) RETURNING id").bind(wid).fetch_one(&pool).await.unwrap();
    sqlx::query("INSERT INTO claims (item_id, guest_id, claimer_name, qty) SELECT $1, guest_id, claimer_name, 1 FROM claims WHERE item_id IN (SELECT id FROM wishlist_items WHERE wishlist_id=$2) LIMIT 1")
        .bind(second).bind(wid).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO wishlist_items (wishlist_id, title, qty_needed, qty_claimed, deleted_at) VALUES ($1, 'gone', 100, 0, now())").bind(wid).execute(&pool).await.unwrap();
    sqlx::query("UPDATE wishlists SET cover_image_key = 'covers/x.jpg', cover_image_status = 'ready', moderation_status = 'ok' WHERE id = $1").bind(wid).execute(&pool).await.unwrap();

    let (_, _, d) = call(
        &pool,
        "GET",
        &format!("/wishlists/{wid}/dashboard"),
        &ck(&tok),
        None,
    )
    .await;
    assert_eq!(d["claims"][0]["claimer_name"], "秘密客");
    assert_eq!(d["totals"]["completion_pct"], 40);

    let (_, _, l) = call(&pool, "GET", "/wishlists", &ck(&tok), None).await;
    assert_eq!(l["data"][0]["completion"]["completion_pct"], 40);
    assert_eq!(l["data"][0]["moderation_status"], "ok");
    assert!(l["data"][0].get("moderation_reason").is_some());

    let (_, _, p) = call(
        &pool,
        "GET",
        &format!("/public/wishlists/{slug}"),
        &[],
        None,
    )
    .await;
    assert_eq!(p["completion"]["completion_pct"], 40);
    assert_eq!(
        p["cover_image_url"],
        wishsync_api::uploads::S3::get().public_url("covers/x.jpg")
    );
    assert_eq!(wishsync_api::wishlists::completion_pct(0, 0), 0);
    assert_all_reconciled(&pool).await;
}
