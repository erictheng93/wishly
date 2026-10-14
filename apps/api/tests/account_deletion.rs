mod common;
use axum::http::StatusCode;
use serde_json::{json, Value};
use sqlx::PgPool;

async fn call(
    pool: &PgPool,
    method: &str,
    uri: &str,
    cookie: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let h = cookie.map(common::ck).unwrap_or_default();
    let (s, _, b) = common::api(pool, method, uri, &h, body).await;
    (s, b)
}

/// 回傳 (user_id, session token)
use common::user_with_staff as user;

use common::claimed_wishlist as wishlist;

#[sqlx::test(migrations = "../../db/migrations")]
async fn account_delete_anonymizes(pool: PgPool) {
    let (uid, tok) = user(&pool, "小米", false).await;
    let (wid, slug, item) = wishlist(&pool, uid, false, 30).await;
    // 此使用者在別人清單上的認領
    let (other, _) = user(&pool, "o", false).await;
    let (_, _, item2) = wishlist(&pool, other, false, 30).await;
    sqlx::query(
        "INSERT INTO claims (item_id, user_id, claimer_name, qty) VALUES ($1, $2, '小米', 1)",
    )
    .bind(item2)
    .bind(uid)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("UPDATE wishlist_items SET qty_claimed=qty_claimed+1 WHERE id=$1")
        .bind(item2)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO auth_identities (user_id, provider, provider_uid) VALUES ($1, 'email', 'a@b.c')").bind(uid).execute(&pool).await.unwrap();
    sqlx::query(
        "INSERT INTO notifications (user_id, channel, kind) VALUES ($1, 'email', 'claim.digest')",
    )
    .bind(uid)
    .execute(&pool)
    .await
    .unwrap();
    let _ = (slug, item);

    assert_eq!(
        call(
            &pool,
            "DELETE",
            "/me",
            Some(&tok),
            Some(json!({"confirm": "yes"}))
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let (s, v) = call(
        &pool,
        "DELETE",
        "/me",
        Some(&tok),
        Some(json!({"confirm": "DELETE"})),
    )
    .await;
    assert_eq!((s, &v["deleted"]), (StatusCode::OK, &json!(true)));

    let (name, email, del, anon): (String, Option<String>, bool, bool) = sqlx::query_as(
        "SELECT display_name, email, deleted_at IS NOT NULL, anonymized_at IS NOT NULL FROM users WHERE id = $1").bind(uid).fetch_one(&pool).await.unwrap();
    assert_eq!(
        (name.as_str(), email, del, anon),
        ("已刪除的使用者", None, true, true)
    );
    let (st, vis): (String, String) =
        sqlx::query_as("SELECT status::text, visibility::text FROM wishlists WHERE id = $1")
            .bind(wid)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!((st.as_str(), vis.as_str()), ("archived", "private"));
    let cn: String = sqlx::query_scalar("SELECT claimer_name FROM claims WHERE user_id = $1")
        .bind(uid)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(cn, "已刪除的使用者");
    let idn: i64 = sqlx::query_scalar("SELECT count(*) FROM auth_identities WHERE user_id = $1")
        .bind(uid)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(idn, 0);
    let pend: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM notifications WHERE user_id = $1 AND status = 'pending'",
    )
    .bind(uid)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(pend, 0);
    // session 失效
    assert_eq!(
        call(&pool, "GET", "/admin/stats", Some(&tok), None).await.0,
        StatusCode::UNAUTHORIZED
    );
    common::assert_claims_reconciled(&pool).await;
}
