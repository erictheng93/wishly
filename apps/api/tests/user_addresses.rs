//! 0069 帳號預設收件地址與居住地址。
mod common;
use axum::http::StatusCode;
use common::{befriend, call, ck, user, wishlist, ListOpts};
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

const SHIP: &str =
    r#"{"recipient_name":"王小明","phone":"0912-345-678","address":"台北市大安區和平東路一段1號"}"#;
fn ship() -> Value {
    serde_json::from_str(SHIP).unwrap()
}
async fn put(pool: &PgPool, tok: &str, kind: &str, b: Value) -> (StatusCode, Value) {
    let (s, _, v) = call(
        pool,
        "PUT",
        &format!("/me/addresses/{kind}"),
        &ck(tok),
        Some(b),
    )
    .await;
    (s, v)
}
async fn handle(pool: &PgPool, id: Uuid) {
    sqlx::query("UPDATE users SET handle='alice' WHERE id=$1")
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn put_get_masks(pool: PgPool) {
    let a = user(&pool, "A").await;
    assert_eq!(
        put(&pool, &a.token, "shipping", ship()).await.0,
        StatusCode::OK
    );
    assert_eq!(
        put(
            &pool,
            &a.token,
            "home",
            json!({"address": "高雄市前鎮區中山路99號"})
        )
        .await
        .0,
        StatusCode::OK
    );
    let (_, _, v) = call(&pool, "GET", "/me/addresses", &ck(&a.token), None).await;
    assert_eq!(v["shipping"]["address"], "台北市大安區***");
    assert!(!v.to_string().contains("0912-345-678") && !v.to_string().contains("王小明"));
    assert_eq!(v["home"]["address"], "高雄市前鎮區***");
    let (s, _, _) = call(&pool, "DELETE", "/me/addresses/home", &ck(&a.token), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (_, _, v) = call(&pool, "GET", "/me/addresses", &ck(&a.token), None).await;
    assert!(v["home"].is_null() && !v["shipping"].is_null());
    assert_eq!(
        put(&pool, &a.token, "shipping", json!({"address": "x"}))
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn home_visible_to_close_only(pool: PgPool) {
    let a = user(&pool, "A").await;
    handle(&pool, a.id).await;
    put(
        &pool,
        &a.token,
        "home",
        json!({"address": "高雄市前鎮區中山路99號"}),
    )
    .await;
    sqlx::query("UPDATE users SET home_address_vis='close' WHERE id=$1")
        .bind(a.id)
        .execute(&pool)
        .await
        .unwrap();
    let f = user(&pool, "F").await;
    let c = user(&pool, "C").await;
    for x in [&f, &c] {
        befriend(&pool, a.id, x.id).await;
    }
    sqlx::query("INSERT INTO close_friends (owner_id, friend_id) VALUES ($1, $2)")
        .bind(a.id)
        .bind(c.id)
        .execute(&pool)
        .await
        .unwrap();
    let get = |t: String| {
        let pool = pool.clone();
        async move { call(&pool, "GET", "/users/alice", &ck(&t), None).await.2 }
    };
    assert!(get(f.token.clone()).await["user"]
        .get("home_address")
        .is_none());
    assert_eq!(
        get(c.token.clone()).await["user"]["home_address"],
        "高雄市前鎮區中山路99號"
    );
    assert_eq!(
        get(a.token.clone()).await["user"]["home_address"],
        "高雄市前鎮區中山路99號"
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn shipping_never_on_profile(pool: PgPool) {
    let a = user(&pool, "A").await;
    handle(&pool, a.id).await;
    put(&pool, &a.token, "shipping", ship()).await;
    for t in [a.token.as_str(), ""] {
        let (_, _, v) = call(&pool, "GET", "/users/alice", &ck(t), None).await;
        let s = v.to_string();
        assert!(
            !s.contains("shipping")
                && !s.contains("0912")
                && !s.contains("和平東路")
                && !s.contains("王小明")
        );
    }
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn from_default_copies_without_plaintext(pool: PgPool) {
    let a = user(&pool, "A").await;
    let w = wishlist(&pool, a.id, ListOpts::default()).await;
    put(&pool, &a.token, "shipping", ship()).await;
    let (s, _, v) = call(
        &pool,
        "POST",
        &format!("/wishlists/{}/shipping-address/from-default", w.id),
        &ck(&a.token),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["has_shipping_address"], true);
    assert!(!v.to_string().contains("0912-345-678"));
    let (n, p, ad): (Vec<u8>, Vec<u8>, Vec<u8>) = sqlx::query_as("SELECT recipient_name_enc, phone_enc, address_enc FROM shipping_addresses WHERE wishlist_id=$1")
        .bind(w.id).fetch_one(&pool).await.unwrap();
    let o = |b: &[u8]| wishly_api::sealed::open_str(b).unwrap();
    assert_eq!(
        (o(&n), o(&p), o(&ad)),
        (
            "王小明".into(),
            "0912-345-678".into(),
            "台北市大安區和平東路一段1號".into()
        )
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn from_default_requires_manage(pool: PgPool) {
    let a = user(&pool, "A").await;
    let b = user(&pool, "B").await;
    let w = wishlist(&pool, a.id, ListOpts::default()).await;
    put(&pool, &b.token, "shipping", ship()).await;
    let (s, _, _) = call(
        &pool,
        "POST",
        &format!("/wishlists/{}/shipping-address/from-default", w.id),
        &ck(&b.token),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn from_default_without_address_409(pool: PgPool) {
    let a = user(&pool, "A").await;
    let w = wishlist(&pool, a.id, ListOpts::default()).await;
    let (s, _, v) = call(
        &pool,
        "POST",
        &format!("/wishlists/{}/shipping-address/from-default", w.id),
        &ck(&a.token),
        None,
    )
    .await;
    assert_eq!(
        (s, v["code"].as_str()),
        (StatusCode::CONFLICT, Some("NO_DEFAULT_ADDRESS"))
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn audit_has_no_address(pool: PgPool) {
    let a = user(&pool, "A").await;
    let w = wishlist(&pool, a.id, ListOpts::default()).await;
    put(&pool, &a.token, "shipping", ship()).await;
    put(
        &pool,
        &a.token,
        "home",
        json!({"address": "高雄市前鎮區中山路99號"}),
    )
    .await;
    call(
        &pool,
        "POST",
        &format!("/wishlists/{}/shipping-address/from-default", w.id),
        &ck(&a.token),
        None,
    )
    .await;
    call(&pool, "DELETE", "/me/addresses/home", &ck(&a.token), None).await;
    let d: Vec<String> = sqlx::query_scalar(
        "SELECT diff::text FROM audit_logs WHERE action LIKE 'profile.address_%'",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(d.len(), 3);
    assert!(d
        .iter()
        .all(|x| !x.contains("中山路") && !x.contains("和平東路") && !x.contains("0912")));
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn account_delete_removes_addresses(pool: PgPool) {
    let a = user(&pool, "A").await;
    put(&pool, &a.token, "shipping", ship()).await;
    let (_, _, e) = call(&pool, "GET", "/me/export", &ck(&a.token), None).await;
    assert_eq!(e["addresses"].as_array().map(|x| x.len()), Some(1));
    let (s, _, _) = call(
        &pool,
        "DELETE",
        "/me",
        &ck(&a.token),
        Some(json!({"confirm": "DELETE"})),
    )
    .await;
    assert!(s.is_success(), "{s}");
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM user_addresses WHERE user_id=$1")
        .bind(a.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 0);
}
