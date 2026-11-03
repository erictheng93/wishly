//! 封鎖與檢舉使用者（0060）：對稱封鎖、不洩漏、pair lock 併發。
mod common;
use axum::http::StatusCode;
use common::{befriend, call, call_app, ck, idem, staff, user, TestUser};
use serde_json::{json, Value};
use sqlx::{PgPool, Postgres, Transaction};
use std::time::Duration;
use uuid::Uuid;
use wishly_api::{app, AppState};

async fn handle(pool: &PgPool, id: Uuid, h: &str) {
    sqlx::query("UPDATE users SET handle=$2 WHERE id=$1")
        .bind(id)
        .bind(h)
        .execute(pool)
        .await
        .unwrap();
}

async fn post(pool: &PgPool, tok: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let (s, _, v) = call(
        pool,
        "POST",
        uri,
        &[ck(tok), idem(Uuid::new_v4())].concat(),
        body,
    )
    .await;
    (s, v)
}

async fn count(pool: &PgPool, sql: &'static str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(pool).await.unwrap()
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn block_clears_relations_and_hides_both_ways(pool: PgPool) {
    let TestUser { id: a, token: ta } = user(&pool, "A").await;
    let TestUser { id: b, token: tb } = user(&pool, "B").await;
    handle(&pool, a, "alice").await;
    handle(&pool, b, "bobby").await;
    befriend(&pool, a, b).await;
    sqlx::query("INSERT INTO friend_requests (from_user, to_user) VALUES ($1,$2)")
        .bind(a)
        .bind(b)
        .execute(&pool)
        .await
        .unwrap();
    let la = common::wishlist(
        &pool,
        a,
        common::ListOpts {
            visibility: Some("selected"),
            ..Default::default()
        },
    )
    .await;
    let lb = common::wishlist(
        &pool,
        b,
        common::ListOpts {
            visibility: Some("friends"),
            ..Default::default()
        },
    )
    .await;
    sqlx::query("INSERT INTO wishlist_allowed_users (wishlist_id, user_id) VALUES ($1,$2)")
        .bind(la.id)
        .bind(b)
        .execute(&pool)
        .await
        .unwrap();
    let ua = format!("/users/{}", "alice");

    // 自己 422、不存在 404
    assert_eq!(
        post(&pool, &ta, &format!("/users/{a}/block"), None).await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        post(
            &pool,
            &ta,
            &format!("/users/{}/block", Uuid::new_v4()),
            None
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );

    let uri = format!("/users/{b}/block");
    assert_eq!(post(&pool, &ta, &uri, None).await.0, StatusCode::NO_CONTENT);
    assert_eq!(post(&pool, &ta, &uri, None).await.0, StatusCode::NO_CONTENT); // 冪等
    assert_eq!(count(&pool, "SELECT count(*) FROM friendships").await, 0);
    assert_eq!(
        count(&pool, "SELECT count(*) FROM friend_requests").await,
        0
    );
    assert_eq!(
        count(&pool, "SELECT count(*) FROM wishlist_allowed_users").await,
        0
    );

    // 雙向都不能送申請，且回應與找不到人相同
    for (tok, to) in [(&ta, "bobby"), (&tb, "alice")] {
        let (s, v) = post(&pool, tok, "/friends/requests", Some(json!({ "to": to }))).await;
        assert_eq!(
            (s, v["code"].as_str()),
            (StatusCode::NOT_FOUND, Some("USER_NOT_FOUND"))
        );
    }
    // 搜尋排除
    for (tok, q) in [(&ta, "bobby"), (&tb, "alice")] {
        let (_, _, v) = call(
            &pool,
            "GET",
            &format!("/friends/search?q={q}"),
            &ck(tok),
            None,
        )
        .await;
        assert_eq!(v["users"].as_array().unwrap().len(), 0, "{v}");
    }
    // 個人頁：被封鎖者 404；封鎖者 relation=blocked 且清空
    assert_eq!(
        call(&pool, "GET", &ua, &ck(&tb), None).await.0,
        StatusCode::NOT_FOUND
    );
    let (s, _, v) = call(&pool, "GET", "/users/bobby", &ck(&ta), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["relation"], "blocked");
    assert_eq!(v["wishlists"], json!([]));
    assert_eq!(v["donations"], json!([]));
    assert!(v["mutual"].is_null() && v["request_id"].is_null() && v["user"]["birthday"].is_null());
    assert_eq!(v["user"]["handle"], "bobby");
    // 受限清單 404（兩個方向）
    sqlx::query("UPDATE wishlists SET status='active' WHERE id = ANY($1)")
        .bind(vec![la.id, lb.id])
        .execute(&pool)
        .await
        .unwrap();
    for (tok, slug) in [(&tb, &la.slug), (&ta, &lb.slug)] {
        let (s, _, _) = call(
            &pool,
            "GET",
            &format!("/restricted/wishlists/{slug}"),
            &ck(tok),
            None,
        )
        .await;
        assert_eq!(s, StatusCode::NOT_FOUND, "{slug}");
    }

    // /me/blocks
    let (_, _, v) = call(&pool, "GET", "/me/blocks", &ck(&ta), None).await;
    assert_eq!(v["users"][0]["id"], b.to_string());
    assert_eq!(v["users"][0]["handle"], "bobby");
    assert!(v["users"][0]["blocked_at"].is_string());
    let (_, _, v) = call(&pool, "GET", "/me/blocks", &ck(&tb), None).await;
    assert_eq!(v["users"], json!([]));

    // 被封鎖者無法解除；封鎖者解除後可再送申請
    let (s, _, _) = call(
        &pool,
        "DELETE",
        &format!("/users/{a}/block"),
        &ck(&tb),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert_eq!(
        post(
            &pool,
            &tb,
            "/friends/requests",
            Some(json!({ "to": "alice" }))
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let (s, _, _) = call(&pool, "DELETE", &uri, &ck(&ta), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert_eq!(
        post(
            &pool,
            &tb,
            "/friends/requests",
            Some(json!({ "to": "alice" }))
        )
        .await
        .0,
        StatusCode::ACCEPTED
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn accept_of_request_fails_after_block(pool: PgPool) {
    let TestUser { id: a, token: ta } = user(&pool, "A").await;
    let TestUser { id: b, token: tb } = user(&pool, "B").await;
    handle(&pool, b, "bobby").await;
    assert_eq!(
        post(
            &pool,
            &ta,
            "/friends/requests",
            Some(json!({ "to": "bobby" }))
        )
        .await
        .0,
        StatusCode::ACCEPTED
    );
    let rid: Uuid = sqlx::query_scalar("SELECT id FROM friend_requests")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        post(&pool, &tb, &format!("/users/{a}/block"), None).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        post(&pool, &tb, &format!("/friends/requests/{rid}/accept"), None)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(count(&pool, "SELECT count(*) FROM friendships").await, 0);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn user_report_create_dedupe_and_admin_shape(pool: PgPool) {
    let TestUser { id: a, token: ta } = user(&pool, "A").await;
    let TestUser { id: b, .. } = user(&pool, "B").await;
    let TestUser { token: st, .. } = staff(&pool, "S").await;
    handle(&pool, b, "bobby").await;
    let uri = format!("/users/{b}/reports");
    assert_eq!(
        post(
            &pool,
            &ta,
            &format!("/users/{a}/reports"),
            Some(json!({"reason":"scam"}))
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        post(&pool, &ta, &uri, Some(json!({"reason":"nope"})))
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        post(
            &pool,
            &ta,
            &uri,
            Some(json!({"reason":"scam","detail":"x".repeat(1001)}))
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        post(
            &pool,
            &ta,
            &format!("/users/{}/reports", Uuid::new_v4()),
            Some(json!({"reason":"scam"}))
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let (s, v1) = post(
        &pool,
        &ta,
        &uri,
        Some(json!({"reason":"scam","detail":"詐騙"})),
    )
    .await;
    assert_eq!(
        (s, v1["status"].as_str()),
        (StatusCode::CREATED, Some("open"))
    );
    let (s, v2) = post(&pool, &ta, &uri, Some(json!({"reason":"other"}))).await;
    assert_eq!((s, &v2["id"]), (StatusCode::CREATED, &v1["id"]));
    assert_eq!(
        count(&pool, "SELECT count(*) FROM content_reports").await,
        1
    );

    // 同時存在清單檢舉
    let owner = user(&pool, "O").await;
    let l = common::wishlist(
        &pool,
        owner.id,
        common::ListOpts {
            visibility: Some("public"),
            ..Default::default()
        },
    )
    .await;
    common::item(&pool, l.id, "x", 1).await;
    let (s, _, _) = call(
        &pool,
        "POST",
        &format!("/public/wishlists/{}/reports", l.slug),
        &ck(&ta),
        Some(json!({"reason":"scam"})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);

    let (s, _, q) = call(&pool, "GET", "/admin/reports", &ck(&st), None).await;
    assert_eq!(s, StatusCode::OK);
    let rows = q["data"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    let ur = rows.iter().find(|r| r["user"].is_object()).unwrap();
    assert!(ur["wishlist"].is_null());
    assert_eq!(
        (ur["user"]["handle"].as_str(), ur["reporter"].as_str()),
        (Some("bobby"), Some("user"))
    );
    let wr = rows.iter().find(|r| r["wishlist"].is_object()).unwrap();
    assert!(wr["user"].is_null());
    // 處理使用者檢舉
    let (s, _, h) = call(
        &pool,
        "PATCH",
        &format!("/admin/reports/{}", ur["id"].as_str().unwrap()),
        &ck(&st),
        Some(json!({"status":"dismissed"})),
    )
    .await;
    assert_eq!(
        (s, h["status"].as_str()),
        (StatusCode::OK, Some("dismissed"))
    );
}

/// 模擬封鎖交易：持有 pair lock 並寫入封鎖列但尚未 commit；回傳 (交易, 後端 pid)
async fn holding_block(
    pool: &PgPool,
    blocker: Uuid,
    blocked: Uuid,
) -> (Transaction<'static, Postgres>, i32) {
    let mut tx = pool.begin().await.unwrap();
    wishly_api::friends::lock_pair_for_test(&mut tx, blocker, blocked).await;
    sqlx::query("INSERT INTO user_blocks (blocker_id, blocked_id) VALUES ($1,$2)")
        .bind(blocker)
        .bind(blocked)
        .execute(&mut *tx)
        .await
        .unwrap();
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    (tx, pid)
}

/// 請求在 pair lock 上等待，封鎖 commit 後才看到封鎖（沒取鎖就會直接成功，wait_for_blocked 逾時）
async fn expect_waits_then_404(
    pool: &PgPool,
    blocker: Uuid,
    blocked: Uuid,
    method: &str,
    uri: String,
    tok: String,
    body: Option<Value>,
) {
    let (tx, pid) = holding_block(pool, blocker, blocked).await;
    let a = app(AppState { pool: pool.clone() });
    let m = method.to_string();
    let h = tokio::spawn(async move {
        call_app(
            &a,
            &m,
            &uri,
            &[ck(&tok), idem(Uuid::new_v4())].concat(),
            body,
        )
        .await
    });
    common::wait_for_blocked(pool, pid, 1).await;
    tx.commit().await.unwrap();
    let (s, _, v) = tokio::time::timeout(Duration::from_secs(10), h)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(s, StatusCode::NOT_FOUND, "{v}");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn request_waits_on_pair_lock_then_sees_block(pool: PgPool) {
    let TestUser { id: a, token: ta } = user(&pool, "A").await;
    let TestUser { id: b, .. } = user(&pool, "B").await;
    handle(&pool, b, "bobby").await;
    expect_waits_then_404(
        &pool,
        b,
        a,
        "POST",
        "/friends/requests".into(),
        ta,
        Some(json!({"to":"bobby"})),
    )
    .await;
    assert_eq!(
        count(&pool, "SELECT count(*) FROM friend_requests").await,
        0
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn accept_waits_on_pair_lock_then_sees_block(pool: PgPool) {
    let TestUser { id: a, .. } = user(&pool, "A").await;
    let TestUser { id: b, token: tb } = user(&pool, "B").await;
    let rid: Uuid = sqlx::query_scalar(
        "INSERT INTO friend_requests (from_user, to_user) VALUES ($1,$2) RETURNING id",
    )
    .bind(a)
    .bind(b)
    .fetch_one(&pool)
    .await
    .unwrap();
    expect_waits_then_404(
        &pool,
        a,
        b,
        "POST",
        format!("/friends/requests/{rid}/accept"),
        tb,
        None,
    )
    .await;
    assert_eq!(count(&pool, "SELECT count(*) FROM friendships").await, 0);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn invite_accept_waits_on_pair_lock_then_sees_block(pool: PgPool) {
    let TestUser { id: a, token: ta } = user(&pool, "A").await;
    let TestUser { id: b, token: tb } = user(&pool, "B").await;
    let (_, _, v) = call(
        &pool,
        "POST",
        "/friends/invites",
        &[ck(&ta), idem(Uuid::new_v4())].concat(),
        None,
    )
    .await;
    let tok = v["token"].as_str().unwrap();
    expect_waits_then_404(
        &pool,
        a,
        b,
        "POST",
        format!("/friends/invites/{tok}/accept"),
        tb,
        None,
    )
    .await;
    assert_eq!(count(&pool, "SELECT count(*) FROM friendships").await, 0);
}
