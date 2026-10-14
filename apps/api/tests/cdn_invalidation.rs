//! Privacy changes must invalidate the actual cached public URL, after the DB commits.
mod common;
use axum::http::StatusCode;
use common::{call, ck, staff, user, wishlist, ListOpts, TestList, TestUser};
use serde_json::json;
use sqlx::PgPool;

#[sqlx::test(migrations = "../../db/migrations")]
async fn privacy_archive_and_moderation_send_cdn_purge(pool: PgPool) {
    let cdn = common::cdn::init();
    let TestUser { id: owner, token } = user(&pool, "Owner").await;
    let TestUser {
        id: _,
        token: staff,
    } = staff(&pool, "Moderator").await;
    let TestList {
        id: privacy,
        slug: privacy_slug,
    } = wishlist(&pool, owner, ListOpts::default()).await;
    let TestList {
        id: archive,
        slug: archive_slug,
    } = wishlist(&pool, owner, ListOpts::default()).await;
    let TestList {
        id: patch_archive,
        slug: patch_archive_slug,
    } = wishlist(&pool, owner, ListOpts::default()).await;
    let TestList {
        id: hidden,
        slug: hidden_slug,
    } = wishlist(&pool, owner, ListOpts::default()).await;
    sqlx::query("UPDATE wishlists SET status='active',visibility='public'")
        .execute(&pool)
        .await
        .unwrap();
    let scenarios = [
        (
            "PATCH",
            privacy,
            privacy_slug.as_str(),
            token.as_str(),
            Some(json!({"visibility":"private"})),
            StatusCode::OK,
        ),
        (
            "DELETE",
            archive,
            archive_slug.as_str(),
            token.as_str(),
            None,
            StatusCode::NO_CONTENT,
        ),
        (
            "PATCH",
            patch_archive,
            patch_archive_slug.as_str(),
            token.as_str(),
            Some(json!({"status":"archived"})),
            StatusCode::OK,
        ),
        (
            "PATCH",
            hidden,
            hidden_slug.as_str(),
            staff.as_str(),
            Some(json!({"moderation_status":"hidden","reason":"Unsafe"})),
            StatusCode::OK,
        ),
    ];
    for (method, id, slug, session, body, expected) in scenarios {
        let uri = if id == hidden {
            format!("/admin/wishlists/{id}/moderation")
        } else {
            format!("/wishlists/{id}")
        };
        let (status, _, response) = call(&pool, method, &uri, &ck(session), body).await;
        assert_eq!(status, expected, "{response}");
        let purge = cdn.wait_for(slug).await;
        assert_eq!(purge.authorization, "Bearer test-purge-token");
        assert_eq!(purge.body, json!({"files":[common::cdn::public_url(slug)]}));
        let state: (String, String, String) = sqlx::query_as("SELECT visibility::text,status::text,moderation_status::text FROM wishlists WHERE id=$1").bind(id).fetch_one(&pool).await.unwrap();
        if id == privacy {
            assert_eq!(state.0, "private");
        } else if id == hidden {
            assert_eq!(state.2, "hidden");
        } else {
            assert_eq!(state.1, "archived");
        }
        let (public_status, _, public) = call(
            &pool,
            "GET",
            &format!("/public/wishlists/{slug}"),
            &[],
            None,
        )
        .await;
        assert_ne!(
            public_status,
            StatusCode::OK,
            "old public content still accessible: {public}"
        );
        assert_eq!(
            cdn.requests_for(slug).len(),
            1,
            "one change unexpectedly sent multiple purges"
        );
    }
    common::assert_all_reconciled(&pool).await;
}
