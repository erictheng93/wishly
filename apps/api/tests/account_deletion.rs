mod common;
use axum::http::StatusCode;
use serde_json::json;
use sqlx::PgPool;

/// 回傳 (user_id, session token)
use common::*;

#[sqlx::test(migrations = "../../db/migrations")]
async fn account_delete_anonymizes(pool: PgPool) {
    let _cdn = common::cdn::init();
    let TestUser {
        id: uid,
        token: tok,
    } = user(&pool, "小米").await;
    let (wid, slug, item) = claimed_wishlist(&pool, uid, false, 30, 2, 1).await;
    sqlx::query("INSERT INTO push_subscriptions (user_id, endpoint, p256dh, auth) VALUES ($1, 'https://fcm.googleapis.com/x', 'k', 'a')")
        .bind(uid).execute(&pool).await.unwrap();
    // 此使用者在別人清單上的捐助
    let TestUser {
        id: other,
        token: _,
    } = user(&pool, "o").await;
    let (_, _, item2) = claimed_wishlist(&pool, other, false, 30, 2, 1).await;
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
    sqlx::query("UPDATE users SET bio='hi', birthday='05-01' WHERE id=$1")
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
            &ck(&tok),
            Some(json!({"confirm": "yes"}))
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let (s, _, v) = call(
        &pool,
        "DELETE",
        "/me",
        &ck(&tok),
        Some(json!({"confirm": "DELETE"})),
    )
    .await;
    assert_eq!((s, &v["deleted"]), (StatusCode::OK, &json!(true)));

    let (name, email, del, anon, personal): (String, Option<String>, bool, bool, bool) = sqlx::query_as(
        "SELECT display_name, email, deleted_at IS NOT NULL, anonymized_at IS NOT NULL, bio IS NOT NULL OR birthday IS NOT NULL FROM users WHERE id = $1").bind(uid).fetch_one(&pool).await.unwrap();
    assert_eq!(
        (name.as_str(), email, del, anon, personal),
        ("已刪除的使用者", None, true, true, false)
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
    let (subs,): (i64,) =
        sqlx::query_as("SELECT count(*) FROM push_subscriptions WHERE user_id = $1")
            .bind(uid)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(subs, 0, "刪帳號後不可再推播到舊裝置");
    // session 失效
    assert_eq!(
        call(&pool, "GET", "/admin/stats", &ck(&tok), None).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn account_delete_purges_public_lists_after_commit(pool: PgPool) {
    let cdn = common::cdn::init();
    let owner = user(&pool, "Owner").await;
    let other = user(&pool, "Other").await;
    let public = wishlist(
        &pool,
        owner.id,
        ListOpts {
            visibility: Some("public"),
            ..Default::default()
        },
    )
    .await;
    let link = wishlist(&pool, owner.id, ListOpts::default()).await;
    let unaffected = wishlist(
        &pool,
        other.id,
        ListOpts {
            visibility: Some("public"),
            ..Default::default()
        },
    )
    .await;
    for list in [&public, &link, &unaffected] {
        item(&pool, list.id, "Visible item", 1).await;
        let (status, _, body) = call(
            &pool,
            "GET",
            &format!("/public/wishlists/{}", list.slug),
            &[],
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["status"], "active");
    }
    let (status, _, body) = call(
        &pool,
        "DELETE",
        "/me",
        &ck(&owner.token),
        Some(json!({"confirm":"DELETE"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["deleted"], true);
    let purge = cdn.wait_for(&public.slug).await;
    assert_eq!(purge.authorization, "Bearer test-purge-token");
    let mut urls: Vec<String> = serde_json::from_value(purge.body["files"].clone()).unwrap();
    urls.sort();
    let mut expected = vec![
        common::cdn::public_url(&public.slug),
        common::cdn::public_url(&link.slug),
    ];
    expected.sort();
    assert_eq!(urls, expected);
    assert!(cdn.requests_for(&unaffected.slug).is_empty());
    for list in [&public, &link] {
        let state: (String, String) =
            sqlx::query_as("SELECT status::text,visibility::text FROM wishlists WHERE id=$1")
                .bind(list.id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(state, ("archived".into(), "private".into()));
        let (status, _, body) = call(
            &pool,
            "GET",
            &format!("/public/wishlists/{}", list.slug),
            &[],
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    }
    assert!(sqlx::query_scalar::<_, bool>(
        "SELECT deleted_at IS NOT NULL AND anonymized_at IS NOT NULL FROM users WHERE id=$1"
    )
    .bind(owner.id)
    .fetch_one(&pool)
    .await
    .unwrap());
    assert_eq!(
        call(&pool, "GET", "/me", &ck(&owner.token), None).await.0,
        StatusCode::UNAUTHORIZED
    );
    let (status, _, body) = call(
        &pool,
        "GET",
        &format!("/public/wishlists/{}", unaffected.slug),
        &[],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "active");
    assert_all_reconciled(&pool).await;
}

/// 刪帳號才結束的清單：他人 reserved 捐助延到 now()+7d 並通知；先前已 closed 的清單不重複寬限。
#[sqlx::test(migrations = "../../db/migrations")]
async fn account_delete_graces_other_claims_only_on_lists_not_already_closed(pool: PgPool) {
    let _cdn = common::cdn::init();
    let owner = user(&pool, "owner").await;
    let claimer = user(&pool, "claimer").await;
    let mut claims = Vec::new();
    for status in ["active", "closed"] {
        let wl = wishlist(&pool, owner.id, Default::default()).await.id;
        sqlx::query("UPDATE wishlists SET status=$2::wishlist_status, closed_at=CASE WHEN $2='closed' THEN now() END WHERE id=$1")
            .bind(wl).bind(status).execute(&pool).await.unwrap();
        let it = item(&pool, wl, "i", 2).await;
        sqlx::query("UPDATE wishlist_items SET qty_claimed=1 WHERE id=$1")
            .bind(it)
            .execute(&pool)
            .await
            .unwrap();
        let c: uuid::Uuid = sqlx::query_scalar("INSERT INTO claims (item_id, user_id, claimer_name, qty, expires_at) VALUES ($1, $2, 'c', 1, now() + interval '2 days') RETURNING id")
            .bind(it).bind(claimer.id).fetch_one(&pool).await.unwrap();
        claims.push(c);
    }
    let (s, _, v) = call(
        &pool,
        "DELETE",
        "/me",
        &ck(&owner.token),
        Some(json!({"confirm": "DELETE"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let days = |c: uuid::Uuid| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, f64>("SELECT (extract(epoch FROM expires_at - now())/86400)::float8 FROM claims WHERE id=$1 AND status='reserved'")
                .bind(c).fetch_one(&pool).await.unwrap()
        }
    };
    let graced = days(claims[0]).await;
    assert!(
        (6.9..=7.0).contains(&graced),
        "active list claim not extended to ~7d: {graced}"
    );
    let untouched = days(claims[1]).await;
    assert!(
        (1.9..=2.0).contains(&untouched),
        "already-closed list claim re-graced: {untouched}"
    );
    let notified: Vec<(Option<String>,)> = sqlx::query_as("SELECT payload->>'claim_id' FROM notifications WHERE user_id=$1 AND kind='claim.list_closed'")
        .bind(claimer.id).fetch_all(&pool).await.unwrap();
    assert_eq!(notified, [(Some(claims[0].to_string()),)]);
    assert_all_reconciled(&pool).await;
}
