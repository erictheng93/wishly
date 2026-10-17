//! Soft-deleted delivered items retain accounting history without inflating live totals.
mod common;
use axum::http::StatusCode;
use common::*;
use serde_json::json;
use sqlx::PgPool;

#[sqlx::test(migrations = "../../db/migrations")]
async fn deleted_delivered_item_is_excluded_from_dashboard_and_export(pool: PgPool) {
    let owner = user(&pool, "owner").await;
    let admin = staff(&pool, "admin").await;
    let list = wishlist(&pool, owner.id, ListOpts::default()).await;
    let removed = item(&pool, list.id, "Delivered and removed", 2).await;
    let live = item(&pool, list.id, "Still available", 4).await;
    let (_, delivered) = user_claim(&pool, removed, 2).await;
    user_claim(&pool, live, 1).await;
    let (status, _, body) = call(
        &pool,
        "PATCH",
        &format!("/claims/{delivered}"),
        &ck(&owner.token),
        Some(json!({"status":"delivered"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, _, before) = call(
        &pool,
        "GET",
        &format!("/wishlists/{}/dashboard", list.id),
        &ck(&owner.token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(before["totals"]["item_count"], 2);
    assert_eq!(before["totals"]["qty_claimed"], 3);
    assert_eq!(before["totals"]["qty_needed"], 6);
    assert_eq!(before["totals"]["fulfilled_count"], 1);
    assert_eq!(before["totals"]["completion_pct"], 50);
    let (status, _, stats) = call(&pool, "GET", "/admin/stats", &ck(&admin.token), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(stats["claims_by_status"]["delivered"], 1);
    assert_eq!(stats["claims_by_status"]["reserved"], 1);

    assert_eq!(
        call(
            &pool,
            "DELETE",
            &format!("/items/{removed}?force=true"),
            &ck(&owner.token),
            None
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    let state: (bool, i32, String, i32, bool) = sqlx::query_as(
        "SELECT i.deleted_at IS NOT NULL,i.qty_claimed,c.status::text,c.qty,c.delivered_at IS NOT NULL FROM wishlist_items i JOIN claims c ON c.item_id=i.id WHERE i.id=$1 AND c.id=$2")
        .bind(removed).bind(delivered).fetch_one(&pool).await.unwrap();
    assert_eq!(state, (true, 2, "delivered".into(), 2, true));
    assert_all_reconciled(&pool).await;

    let (status, _, dashboard) = call(
        &pool,
        "GET",
        &format!("/wishlists/{}/dashboard", list.id),
        &ck(&owner.token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        dashboard["totals"]["item_count"], 1,
        "dashboard included soft-deleted delivered item"
    );
    assert_eq!(dashboard["totals"]["qty_claimed"], 1);
    assert_eq!(dashboard["totals"]["qty_needed"], 4);
    assert_eq!(dashboard["totals"]["fulfilled_count"], 0);
    assert_eq!(dashboard["totals"]["completion_pct"], 25);
    let items = dashboard["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["item_id"], live.to_string());

    // Admin statistics count historical claim records, not retained units.
    let (status, _, stats) = call(&pool, "GET", "/admin/stats", &ck(&admin.token), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(stats["claims_by_status"]["delivered"], 1);
    assert_eq!(stats["claims_by_status"]["reserved"], 1);
    assert_eq!(stats["users"], 4); // owner + admin + 兩位會員捐助者
    assert_eq!(stats["wishlists"], 1);
    assert_eq!(stats["active_wishlists"], 1);
    assert!(stats.get("qty_claimed").is_none());

    let (status, _, export) = call(&pool, "GET", "/me/export", &ck(&owner.token), None).await;
    assert_eq!(status, StatusCode::OK);
    let lists = export["wishlists"].as_array().unwrap();
    assert_eq!(lists.len(), 1);
    assert_eq!(lists[0]["id"], list.id.to_string());
    let items = lists[0]["items"].as_array().unwrap();
    assert_eq!(
        items.len(),
        1,
        "export included soft-deleted delivered item"
    );
    assert_eq!(items[0]["id"], live.to_string());
    assert_eq!(items[0]["qty_claimed"], 1);
    assert_eq!(items[0]["qty_needed"], 4);
    assert_all_reconciled(&pool).await;
}
