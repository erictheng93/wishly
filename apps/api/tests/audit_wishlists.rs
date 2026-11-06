//! 清單 / 品項寫入的稽核：動作名稱、diff 形狀，且不含收件資訊。
mod common;
use axum::http::StatusCode;
use common::{befriend, call, ck, item, user, wishlist, ListOpts};
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

async fn logs(pool: &PgPool, action: &str) -> Vec<Value> {
    sqlx::query_scalar(
        "SELECT diff FROM audit_logs WHERE action = $1 AND actor_type = 'user' ORDER BY id",
    )
    .bind(action)
    .fetch_all(pool)
    .await
    .unwrap()
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn wishlist_create_update_archive(pool: PgPool) {
    let me = user(&pool, "甲").await;
    let (s, _, b) = call(
        &pool,
        "POST",
        "/wishlists",
        &ck(&me.token),
        Some(json!({ "type": "registry", "title": "生日", "visibility": "public" })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let id = b["id"].as_str().unwrap().to_string();
    assert_eq!(
        logs(&pool, "wishlist.create").await[0]["after"]["title"],
        "生日"
    );

    let url = format!("/wishlists/{id}");
    let long = "字".repeat(300);
    let (s, _, _) = call(
        &pool,
        "PATCH",
        &url,
        &ck(&me.token),
        Some(json!({ "title": "生日", "description": long, "visibility": "friends" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let l = logs(&pool, "wishlist.update").await;
    assert_eq!(l.len(), 1);
    let ch = l[0]["changed"].as_object().unwrap();
    assert_eq!(ch.len(), 2, "title 沒變不該出現: {ch:?}");
    assert_eq!(
        ch["visibility"],
        json!({ "before": "public", "after": "friends" })
    );
    assert_eq!(
        ch["description"]["after"].as_str().unwrap().chars().count(),
        200
    );

    // 無變動 → 不寫
    let (s, _, _) = call(
        &pool,
        "PATCH",
        &url,
        &ck(&me.token),
        Some(json!({ "title": "生日" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(logs(&pool, "wishlist.update").await.len(), 1);

    let (s, _, _) = call(&pool, "DELETE", &url, &ck(&me.token), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert_eq!(
        logs(&pool, "wishlist.archive").await[0]["before"]["title"],
        "生日"
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn item_create_update_delete_reorder(pool: PgPool) {
    let me = user(&pool, "甲").await;
    let wl = wishlist(&pool, me.id, ListOpts::default()).await;
    let (s, _, b) = call(
        &pool,
        "POST",
        &format!("/wishlists/{}/items", wl.id),
        &ck(&me.token),
        Some(json!({ "title": "咖啡機", "qty_needed": 2 })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let iid = b["id"].as_str().unwrap().to_string();
    let a = &logs(&pool, "item.create").await[0]["after"];
    assert_eq!(
        (a["title"].as_str(), a["qty_needed"].as_i64()),
        (Some("咖啡機"), Some(2))
    );

    let (s, _, b) = call(
        &pool,
        "PATCH",
        &format!("/items/{iid}"),
        &ck(&me.token),
        Some(json!({ "title": "咖啡機", "qty_needed": 3 })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let l = logs(&pool, "item.update").await;
    assert_eq!(
        l[0]["changed"],
        json!({ "qty_needed": { "before": 2, "after": 3 } })
    );
    call(
        &pool,
        "PATCH",
        &format!("/items/{iid}"),
        &ck(&me.token),
        Some(json!({ "qty_needed": 3 })),
    )
    .await;
    assert_eq!(logs(&pool, "item.update").await.len(), 1);

    let other = item(&pool, wl.id, "杯子", 1).await;
    let ids: Vec<Uuid> = vec![other, iid.parse().unwrap()];
    let (s, _, b) = call(
        &pool,
        "POST",
        &format!("/wishlists/{}/items/reorder", wl.id),
        &ck(&me.token),
        Some(json!({ "item_ids": ids })),
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{b}");
    assert_eq!(logs(&pool, "item.reorder").await[0], json!({ "count": 2 }));

    let (s, _, _) = call(
        &pool,
        "DELETE",
        &format!("/items/{iid}"),
        &ck(&me.token),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert_eq!(
        logs(&pool, "item.delete").await[0]["before"]["title"],
        "咖啡機"
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn address_and_close_friends_are_not_leaked(pool: PgPool) {
    let me = user(&pool, "甲").await;
    let f1 = user(&pool, "乙").await;
    let f2 = user(&pool, "丙").await;
    befriend(&pool, me.id, f1.id).await;
    befriend(&pool, me.id, f2.id).await;
    let wl = wishlist(&pool, me.id, ListOpts::default()).await;

    let (s, _, b) = call(&pool, "PUT", &format!("/wishlists/{}/shipping-address", wl.id), &ck(&me.token),
        Some(json!({ "recipient_name": "王小明", "phone": "0912345678", "address": "台北市信義路五段七號" }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let l = logs(&pool, "wishlist.shipping_address.update").await;
    assert_eq!(l, vec![json!({ "changed": true })]);

    for (m, f) in [("PUT", f1.id), ("PUT", f2.id), ("DELETE", f1.id)] {
        let (s, _, b) = call(
            &pool,
            m,
            &format!("/me/close-friends/{f}"),
            &ck(&me.token),
            None,
        )
        .await;
        assert_eq!(s, StatusCode::NO_CONTENT, "{b}");
    }
    assert_eq!(
        logs(&pool, "close_friends.add").await,
        vec![json!({}), json!({})]
    );
    assert_eq!(logs(&pool, "close_friends.remove").await, vec![json!({})]);
}
