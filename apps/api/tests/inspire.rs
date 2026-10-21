//! 靈感頁 GET /inspire/{ref}/wishes 與品項建立時的 catalog_ref
mod common;
use axum::http::StatusCode;
use common::*;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

async fn tagged(pool: &PgPool, owner: Uuid, vis: &'static str, title: &str) -> Uuid {
    let wl = wishlist(
        pool,
        owner,
        ListOpts {
            visibility: Some(vis),
            ..Default::default()
        },
    )
    .await;
    let it = item(pool, wl.id, title, 1).await;
    sqlx::query("UPDATE wishlist_items SET catalog_ref='cup-1' WHERE id=$1")
        .bind(it)
        .execute(pool)
        .await
        .unwrap();
    wl.id
}

async fn titles(pool: &PgPool, q: &str, h: &[(&str, String)]) -> Vec<String> {
    let (s, hd, v) = call(pool, "GET", &format!("/inspire/cup-1/wishes{q}"), h, None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(hd["cache-control"], "no-store");
    let mut t: Vec<String> = v["wishes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["item"]["title"].as_str().unwrap().to_string())
        .collect();
    t.sort();
    t
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn visibility_rules(pool: PgPool) {
    let me = user(&pool, "我").await;
    let f = user(&pool, "友").await;
    let x = user(&pool, "路人").await;
    befriend(&pool, me.id, f.id).await;
    for v in ["public", "friends", "private"] {
        tagged(&pool, f.id, v, v).await;
    }
    let sel = tagged(&pool, f.id, "selected", "selected").await;
    tagged(&pool, x.id, "friends", "x-friends").await; // 非好友
    tagged(&pool, me.id, "public", "mine").await; // 自己的不列
                                                  // 同一商品但另一代號不應出現
    let other = wishlist(&pool, f.id, ListOpts::default()).await;
    item(&pool, other.id, "other-ref", 1).await;

    // 訪客：只有 public
    assert_eq!(titles(&pool, "", &[]).await, ["mine", "public"]);
    // 好友 me：public + friends，selected 尚未被選中
    assert_eq!(
        titles(&pool, "", &ck(&me.token)).await,
        ["friends", "public"]
    );
    sqlx::query("INSERT INTO wishlist_allowed_users (wishlist_id, user_id) VALUES ($1,$2)")
        .bind(sel)
        .bind(me.id)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        titles(&pool, "", &ck(&me.token)).await,
        ["friends", "public", "selected"]
    );
    // scope 過濾
    assert_eq!(
        titles(&pool, "?scope=friends", &ck(&me.token)).await,
        ["friends"]
    );
    assert_eq!(
        titles(&pool, "?scope=selected", &ck(&me.token)).await,
        ["selected"]
    );
    // 非好友 x：看得到 public（f 的），看不到 friends/selected
    assert_eq!(titles(&pool, "", &ck(&x.token)).await, ["mine", "public"]);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn bad_ref_and_scope_rejected(pool: PgPool) {
    let (s, _, _) = call(&pool, "GET", "/inspire/a%20b/wishes", &[], None).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    let (s, _, _) = call(
        &pool,
        "GET",
        "/inspire/cup-1/wishes?scope=private",
        &[],
        None,
    )
    .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn create_stores_catalog_ref_and_rejects_bad(pool: PgPool) {
    let o = user(&pool, "主").await;
    let wl = wishlist(&pool, o.id, ListOpts::default()).await;
    let uri = format!("/wishlists/{}/items", wl.id);
    let (s, _, v) = call(
        &pool,
        "POST",
        &uri,
        &ck(&o.token),
        Some(json!({"title":"杯","catalog_ref":"cup-1"})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let id: Uuid = v["id"].as_str().unwrap().parse().unwrap();
    let r: Option<String> =
        sqlx::query_scalar("SELECT catalog_ref FROM wishlist_items WHERE id=$1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(r.as_deref(), Some("cup-1"));
    let (s, _, _) = call(
        &pool,
        "POST",
        &uri,
        &ck(&o.token),
        Some(json!({"title":"杯","catalog_ref":"a b!"})),
    )
    .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    // PATCH 不改
    let (s, _, v) = call(
        &pool,
        "PATCH",
        &format!("/items/{id}"),
        &ck(&o.token),
        Some(json!({"title":"杯2","catalog_ref":"zzz"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let r: Option<String> =
        sqlx::query_scalar("SELECT catalog_ref FROM wishlist_items WHERE id=$1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(r.as_deref(), Some("cup-1"));
}
