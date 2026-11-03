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

#[sqlx::test(migrations = "../../db/migrations")]
async fn profile_without_handle_is_reachable_by_user_id(pool: PgPool) {
    let owner = user(&pool, "No Handle").await.id;
    let h = ck(&user(&pool, "Viewer").await.token);
    let get = |path: String, headers: Vec<(&'static str, String)>| {
        let pool = pool.clone();
        async move { call(&pool, "GET", &path, &headers, None).await }
    };
    // 未登入用 id 看不到（與不存在相同）
    assert_eq!(
        get(format!("/users/{owner}"), vec![]).await.0,
        StatusCode::NOT_FOUND
    );
    let (status, _, profile) = get(format!("/users/{owner}"), h.clone()).await;
    assert_eq!(status, StatusCode::OK, "{profile}");
    assert_eq!(profile["user"]["id"], json!(owner));
    assert!(profile["user"]["handle"].is_null());
    assert_eq!(profile["relation"], "none");

    // 有 handle 的人用 id 也進得去，回傳真正的 handle
    sqlx::query("UPDATE users SET handle='has_handle' WHERE id=$1")
        .bind(owner)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        get(format!("/users/{owner}"), h.clone()).await.2["user"]["handle"],
        "has_handle"
    );

    // 停權帳號與不存在的 id 都是 404
    sqlx::query("UPDATE users SET blocked_at=now() WHERE id=$1")
        .bind(owner)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        get(format!("/users/{owner}"), h.clone()).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get(format!("/users/{}", uuid::Uuid::new_v4()), h).await.0,
        StatusCode::NOT_FOUND
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn profile_avatar_request_id_and_mutual_friends(pool: PgPool) {
    let TestUser {
        id: owner,
        token: owner_token,
    } = user(&pool, "Owner").await;
    let viewer = user(&pool, "Viewer").await;
    let h = ck(&viewer.token);
    let make_friends = |a: uuid::Uuid, b: uuid::Uuid| {
        let pool = pool.clone();
        async move {
            sqlx::query("INSERT INTO friendships (user_a, user_b) VALUES (LEAST($1::uuid,$2::uuid), GREATEST($1::uuid,$2::uuid))")
                .bind(a).bind(b).execute(&pool).await.unwrap();
        }
    };
    let get = |h: Vec<(&'static str, String)>| {
        let pool = pool.clone();
        async move { call(&pool, "GET", &format!("/users/{owner}"), &h, None).await }
    };

    // avatar_url：null / 有值
    let p = get(h.clone()).await.2;
    assert!(p["user"]["avatar_url"].is_null());
    sqlx::query("UPDATE users SET avatar_key='avatars/00000000-0000-4000-8000-000000000001.webp' WHERE id=$1")
        .bind(owner)
        .execute(&pool)
        .await
        .unwrap();
    assert!(get(h.clone()).await.2["user"]["avatar_url"].is_string());

    // none：request_id null；匿名需 handle，這裡用 id 的匿名是 404，故改設 handle
    sqlx::query("UPDATE users SET handle='owner_x' WHERE id=$1")
        .bind(owner)
        .execute(&pool)
        .await
        .unwrap();
    let p = get(h.clone()).await.2;
    assert_eq!(
        (p["relation"].as_str(), p["request_id"].is_null()),
        (Some("none"), true)
    );
    let (_, _, anon) = call(&pool, "GET", "/users/owner_x", &[], None).await;
    assert!(anon["request_id"].is_null() && anon["mutual"].is_null());

    // outgoing / incoming 各回對應申請的 id
    let out_id: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO friend_requests (from_user, to_user) VALUES ($1,$2) RETURNING id",
    )
    .bind(viewer.id)
    .bind(owner)
    .fetch_one(&pool)
    .await
    .unwrap();
    let p = get(h.clone()).await.2;
    assert_eq!(p["relation"], "outgoing");
    assert_eq!(p["request_id"], json!(out_id));
    sqlx::query("DELETE FROM friend_requests")
        .execute(&pool)
        .await
        .unwrap();
    let in_id: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO friend_requests (from_user, to_user) VALUES ($1,$2) RETURNING id",
    )
    .bind(owner)
    .bind(viewer.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    let p = get(h.clone()).await.2;
    assert_eq!(p["relation"], "incoming");
    assert_eq!(p["request_id"], json!(in_id));
    sqlx::query("DELETE FROM friend_requests")
        .execute(&pool)
        .await
        .unwrap();

    // mutual：3 位共同好友 + 1 位只與本人是好友 + 1 位已刪除的共同好友
    let mut shared = vec![];
    for n in ["A", "B", "C", "D"] {
        let u = user(&pool, &format!("Mutual {n}")).await.id;
        make_friends(u, owner).await;
        make_friends(u, viewer.id).await;
        shared.push(u);
    }
    let only_owner = user(&pool, "Only Owner").await.id;
    make_friends(only_owner, owner).await;
    sqlx::query("UPDATE users SET deleted_at=now() WHERE id=$1")
        .bind(shared[3])
        .execute(&pool)
        .await
        .unwrap();
    make_friends(viewer.id, owner).await;
    let p = get(h.clone()).await.2;
    assert_eq!(p["relation"], "friend");
    assert!(p["request_id"].is_null());
    assert_eq!(p["mutual"]["count"], 3);
    let names: Vec<_> = p["mutual"]["users"]
        .as_array()
        .unwrap()
        .iter()
        .map(|u| u["display_name"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(names, ["Mutual A", "Mutual B", "Mutual C"]);
    assert!(p["mutual"]["users"][0]["avatar_url"].is_null());

    // 超過 3 位只取 3 位、count 為總數；本人與匿名為 null
    let extra = user(&pool, "Mutual E").await.id;
    make_friends(extra, owner).await;
    make_friends(extra, viewer.id).await;
    let p = get(h.clone()).await.2;
    assert_eq!(
        (
            p["mutual"]["count"].clone(),
            p["mutual"]["users"].as_array().unwrap().len()
        ),
        (json!(4), 3)
    );
    let own = call(&pool, "GET", "/users/owner_x", &ck(&owner_token), None)
        .await
        .2;
    assert!(own["relation"] == "self" && own["mutual"].is_null());
    let (_, _, anon) = call(&pool, "GET", "/users/owner_x", &[], None).await;
    assert!(anon["mutual"].is_null());
}
