//! Dedicated public profile boundaries: handles are case insensitive; hidden data stays hidden.
mod common;
use axum::http::StatusCode;
use common::{call, ck, user, wishlist, ListOpts, TestList, TestUser};
use serde_json::json;
use sqlx::PgPool;

#[sqlx::test(migrations = "../../db/migrations")]
async fn profile_handle_case_and_unpublished_fields_are_private(pool: PgPool) {
    let TestUser {
        id: owner,
        token: session,
    } = user(&pool, "Owner").await;
    sqlx::query("UPDATE users SET handle='owner_01' WHERE id=$1")
        .bind(owner)
        .execute(&pool)
        .await
        .unwrap();
    let TestList {
        id: visible,
        slug: visible_slug,
    } = wishlist(&pool, owner, ListOpts::default()).await;
    sqlx::query("UPDATE wishlists SET visibility='public',title='Visible' WHERE id=$1")
        .bind(visible)
        .execute(&pool)
        .await
        .unwrap();
    for (visibility, status, moderation) in [
        ("private", "active", "ok"),
        ("public", "draft", "ok"),
        ("public", "archived", "ok"),
        ("public", "active", "hidden"),
    ] {
        let id = wishlist(&pool, owner, ListOpts::default()).await.id;
        sqlx::query("UPDATE wishlists SET title='Secret',description='Secret description',visibility=$2::visibility,status=$3::wishlist_status,moderation_status=$4::moderation_status,moderation_reason=CASE WHEN $4='hidden' THEN 'Secret reason' END WHERE id=$1")
            .bind(id).bind(visibility).bind(status).bind(moderation).execute(&pool).await.unwrap();
    }
    let deleted = wishlist(&pool, owner, ListOpts::default()).await.id;
    sqlx::query("UPDATE wishlists SET visibility='public',title='Secret deleted',deleted_at=now() WHERE id=$1").bind(deleted).execute(&pool).await.unwrap();
    for (handle, headers) in [
        ("owner_01", vec![]),
        ("OWNER_01", vec![]),
        ("OwNeR_01", ck(&session)),
    ] {
        let (status, _, profile) =
            call(&pool, "GET", &format!("/users/{handle}"), &headers, None).await;
        assert_eq!(status, StatusCode::OK, "{profile}");
        assert_eq!(profile["user"]["handle"], "owner_01");
        assert_eq!(profile["user"]["id"], json!(owner));
        assert_eq!(profile["wishlists"].as_array().unwrap().len(), 1);
        assert_eq!(profile["wishlists"][0]["slug"], visible_slug);
        for field in ["email", "notification_prefs", "deleted_at", "is_staff"] {
            assert!(profile["user"].get(field).is_none(), "{field} leaked");
        }
        assert!(!profile.to_string().contains("Secret"), "{profile}");
        assert!(profile["donations"].as_array().unwrap().is_empty());
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM wishlists WHERE owner_id=$1")
            .bind(owner)
            .fetch_one(&pool)
            .await
            .unwrap(),
        6
    );
    common::assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn deleted_account_profile_is_indistinguishable_from_missing_handle(pool: PgPool) {
    let TestUser {
        id: owner,
        token: _,
    } = user(&pool, "Deleted Name").await;
    sqlx::query("UPDATE users SET handle='deleted_owner',deleted_at=now() WHERE id=$1")
        .bind(owner)
        .execute(&pool)
        .await
        .unwrap();
    let id = wishlist(&pool, owner, ListOpts::default()).await.id;
    sqlx::query("UPDATE wishlists SET visibility='public' WHERE id=$1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    let missing = call(&pool, "GET", "/users/missing_handle", &[], None).await;
    for handle in ["deleted_owner", "DELETED_OWNER"] {
        let response = call(&pool, "GET", &format!("/users/{handle}"), &[], None).await;
        assert_eq!(response.0, StatusCode::NOT_FOUND);
        assert_eq!(response.2, missing.2);
        assert!(!response.2.to_string().contains("Deleted Name"));
    }
    assert!(
        sqlx::query_scalar::<_, bool>("SELECT deleted_at IS NOT NULL FROM users WHERE id=$1")
            .bind(owner)
            .fetch_one(&pool)
            .await
            .unwrap()
    );
    common::assert_all_reconciled(&pool).await;
}
