//! 好友邀請連結附帶清單：建立規則、預覽、接受（selected 自動入名單）、撤銷範圍
mod common;
use axum::http::StatusCode;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

async fn call(
    pool: &PgPool,
    method: &str,
    uri: &str,
    tok: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let (s, _, v) = common::call(
        pool,
        method,
        uri,
        &[common::ck(tok), common::idem(Uuid::new_v4())].concat(),
        body,
    )
    .await;
    (s, v)
}

async fn list(pool: &PgPool, owner: Uuid, vis: &'static str) -> common::TestList {
    common::wishlist(
        pool,
        owner,
        common::ListOpts {
            visibility: Some(vis),
            ..Default::default()
        },
    )
    .await
}

async fn invite(pool: &PgPool, tok: &str, wid: Option<Uuid>) -> (StatusCode, Value) {
    call(
        pool,
        "POST",
        "/friends/invites",
        tok,
        wid.map(|w| json!({ "wishlist_id": w })),
    )
    .await
}

async fn preview(pool: &PgPool, token: &str) -> (StatusCode, Value) {
    let (s, _, v) =
        common::call(pool, "GET", &format!("/friends/invites/{token}"), &[], None).await;
    (s, v)
}

async fn accept(pool: &PgPool, tok: &str, token: &str) -> (StatusCode, Value) {
    call(
        pool,
        "POST",
        &format!("/friends/invites/{token}/accept"),
        tok,
        None,
    )
    .await
}

fn token(v: &Value) -> String {
    v["token"].as_str().unwrap().to_string()
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn general_and_list_invites_do_not_revoke_each_other(pool: PgPool) {
    let a = common::user(&pool, "A").await;
    let l = list(&pool, a.id, "friends").await;
    let (s, g1) = invite(&pool, &a.token, None).await;
    assert_eq!(s, StatusCode::CREATED);
    assert!(g1["wishlist"].is_null());
    let (s, p1) = invite(&pool, &a.token, Some(l.id)).await;
    assert_eq!(s, StatusCode::CREATED, "{p1}");
    assert_eq!(
        p1["wishlist"],
        json!({ "id": l.id, "slug": l.slug, "title": "清單" })
    );
    // 再建一般邀請：舊一般失效、清單邀請仍有效
    let (_, g2) = invite(&pool, &a.token, None).await;
    assert_eq!(preview(&pool, &token(&g1)).await.0, StatusCode::NOT_FOUND);
    assert_eq!(preview(&pool, &token(&g2)).await.0, StatusCode::OK);
    assert_eq!(preview(&pool, &token(&p1)).await.0, StatusCode::OK);
    // 再建清單邀請：舊清單邀請失效、一般邀請仍有效
    let (_, p2) = invite(&pool, &a.token, Some(l.id)).await;
    assert_eq!(preview(&pool, &token(&p1)).await.0, StatusCode::NOT_FOUND);
    assert_eq!(preview(&pool, &token(&p2)).await.0, StatusCode::OK);
    assert_eq!(preview(&pool, &token(&g2)).await.0, StatusCode::OK);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn list_invite_requires_own_restricted_list(pool: PgPool) {
    let a = common::user(&pool, "A").await;
    let b = common::user(&pool, "B").await;
    let theirs = list(&pool, b.id, "friends").await;
    let (s, v) = invite(&pool, &a.token, Some(theirs.id)).await;
    assert_eq!(
        (s, v["code"].as_str()),
        (StatusCode::NOT_FOUND, Some("NOT_FOUND"))
    );
    assert_eq!(
        invite(&pool, &a.token, Some(Uuid::new_v4())).await.0,
        StatusCode::NOT_FOUND
    );
    for vis in ["public", "private"] {
        let l = list(&pool, a.id, vis).await;
        let (s, v) = invite(&pool, &a.token, Some(l.id)).await;
        assert_eq!(
            (s, v["code"].as_str()),
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                Some("LIST_NOT_RESTRICTED")
            ),
            "{vis}"
        );
    }
    let gone = list(&pool, a.id, "friends").await;
    sqlx::query("UPDATE wishlists SET deleted_at = now() WHERE id = $1")
        .bind(gone.id)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        invite(&pool, &a.token, Some(gone.id)).await.0,
        StatusCode::NOT_FOUND
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn friends_list_invite_befriends_and_opens_list(pool: PgPool) {
    let a = common::user(&pool, "A").await;
    let b = common::user(&pool, "B").await;
    let l = list(&pool, a.id, "friends").await;
    common::item(&pool, l.id, "奶瓶", 1).await;
    let (_, inv) = invite(&pool, &a.token, Some(l.id)).await;
    let t = token(&inv);
    let (s, p) = preview(&pool, &t).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(
        p["wishlist"],
        json!({ "slug": l.slug, "title": "清單", "visibility": "friends" })
    );
    let uri = format!("/restricted/wishlists/{}", l.slug);
    assert_ne!(
        common::call(&pool, "GET", &uri, &common::ck(&b.token), None)
            .await
            .0,
        StatusCode::OK
    );
    let (s, v) = accept(&pool, &b.token, &t).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["wishlist"], json!({ "slug": l.slug }));
    assert_eq!(
        common::call(&pool, "GET", &uri, &common::ck(&b.token), None)
            .await
            .0,
        StatusCode::OK
    );
    // friends 清單不寫名單
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM wishlist_allowed_users")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 0);
    // 清單改為 private：預覽退化為一般邀請（wishlist null）、接受不回 wishlist
    sqlx::query("UPDATE wishlists SET visibility = 'private' WHERE id = $1")
        .bind(l.id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(preview(&pool, &t).await.1["wishlist"].is_null());
    assert!(accept(&pool, &b.token, &t).await.1["wishlist"].is_null());
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn selected_list_invite_adds_accepter_even_if_already_friends(pool: PgPool) {
    let a = common::user(&pool, "A").await;
    let b = common::user(&pool, "B").await;
    common::befriend(&pool, a.id, b.id).await;
    let l = list(&pool, a.id, "selected").await;
    let uri = format!("/restricted/wishlists/{}", l.slug);
    assert_ne!(
        common::call(&pool, "GET", &uri, &common::ck(&b.token), None)
            .await
            .0,
        StatusCode::OK
    );
    let (_, inv) = invite(&pool, &a.token, Some(l.id)).await;
    let t = token(&inv);
    assert_eq!(
        preview(&pool, &t).await.1["wishlist"]["visibility"],
        "selected"
    );
    for _ in 0..2 {
        let (s, v) = accept(&pool, &b.token, &t).await;
        assert_eq!(
            (s, &v["wishlist"]),
            (StatusCode::OK, &json!({ "slug": l.slug })),
            "repeat accept must stay idempotent"
        );
    }
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM wishlist_allowed_users WHERE wishlist_id = $1 AND user_id = $2",
    )
    .bind(l.id)
    .bind(b.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(n, 1, "accepter must be in allowed users");
    assert_eq!(
        common::call(&pool, "GET", &uri, &common::ck(&b.token), None)
            .await
            .0,
        StatusCode::OK
    );
    // 自己接受自己的清單邀請仍是 SELF_INVITE
    assert_eq!(accept(&pool, &a.token, &t).await.1["code"], "SELF_INVITE");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn delete_with_wishlist_id_revokes_only_that_invite(pool: PgPool) {
    let a = common::user(&pool, "A").await;
    let l1 = list(&pool, a.id, "friends").await;
    let l2 = list(&pool, a.id, "selected").await;
    let t = |v: &(StatusCode, Value)| token(&v.1);
    let g = invite(&pool, &a.token, None).await;
    let i1 = invite(&pool, &a.token, Some(l1.id)).await;
    let i2 = invite(&pool, &a.token, Some(l2.id)).await;
    let uri = format!("/friends/invites?wishlist_id={}", l1.id);
    assert_eq!(
        call(&pool, "DELETE", &uri, &a.token, None).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(preview(&pool, &t(&i1)).await.0, StatusCode::NOT_FOUND);
    assert_eq!(preview(&pool, &t(&i2)).await.0, StatusCode::OK);
    assert_eq!(preview(&pool, &t(&g)).await.0, StatusCode::OK);
    // 不帶 query：全部撤銷
    assert_eq!(
        call(&pool, "DELETE", "/friends/invites", &a.token, None)
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(preview(&pool, &t(&i2)).await.0, StatusCode::NOT_FOUND);
    assert_eq!(preview(&pool, &t(&g)).await.0, StatusCode::NOT_FOUND);
}
