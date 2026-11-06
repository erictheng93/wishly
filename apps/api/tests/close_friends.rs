//! 帳號摯友名單（0067）：API、selected 清單可見性、解除好友 / 封鎖清除、migration 合併。
mod common;
use axum::http::StatusCode;
use common::{befriend, call, ck, idem, user, wishlist, ListOpts};
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

async fn req(
    pool: &PgPool,
    m: &str,
    uri: &str,
    tok: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let (s, _, v) = call(
        pool,
        m,
        uri,
        &[ck(tok), idem(Uuid::new_v4())].concat(),
        body,
    )
    .await;
    (s, v)
}

async fn close(pool: &PgPool, o: Uuid, f: Uuid) {
    sqlx::query("INSERT INTO close_friends (owner_id, friend_id) VALUES ($1, $2)")
        .bind(o)
        .bind(f)
        .execute(pool)
        .await
        .unwrap();
}

async fn n_close(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM close_friends")
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn put_requires_friendship(pool: PgPool) {
    let a = user(&pool, "A").await;
    let b = user(&pool, "B").await;
    let u = format!("/me/close-friends/{}", b.id);
    let (s, v) = req(&pool, "PUT", &u, &a.token, None).await;
    assert_eq!(
        (s, v["code"].as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, Some("NOT_FRIEND"))
    );
    befriend(&pool, a.id, b.id).await;
    assert_eq!(
        req(&pool, "PUT", &u, &a.token, None).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        req(&pool, "PUT", &u, &a.token, None).await.0,
        StatusCode::NO_CONTENT
    );
    let (s, v) = req(&pool, "GET", "/me/close-friends", &a.token, None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["users"].as_array().unwrap().len(), 1);
    assert_eq!(v["users"][0]["id"], json!(b.id));
    // 單向：B 的名單仍為空
    assert_eq!(
        req(&pool, "GET", "/me/close-friends", &b.token, None)
            .await
            .1["users"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    // 對自己 422
    let me = format!("/me/close-friends/{}", a.id);
    assert_eq!(
        req(&pool, "PUT", &me, &a.token, None).await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        req(&pool, "DELETE", &u, &a.token, None).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(n_close(&pool).await, 0);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn selected_list_visible_to_close_only(pool: PgPool) {
    let a = user(&pool, "A").await;
    let b = user(&pool, "B").await;
    let c = user(&pool, "C").await;
    let d = user(&pool, "D").await;
    for f in [&b, &c] {
        befriend(&pool, a.id, f.id).await;
    }
    befriend(&pool, c.id, d.id).await;
    let w = wishlist(
        &pool,
        a.id,
        ListOpts {
            visibility: Some("selected"),
            ..Default::default()
        },
    )
    .await;
    let uri = format!("/restricted/wishlists/{}", w.slug);
    let status = |t: String| {
        let (pool, uri) = (pool.clone(), uri.clone());
        async move { call(&pool, "GET", &uri, &ck(&t), None).await.0 }
    };
    assert_eq!(status(b.token.clone()).await, StatusCode::FORBIDDEN);
    // C 只是 D 的摯友，不是 A 的
    close(&pool, d.id, c.id).await;
    assert_eq!(
        status(c.token.clone()).await,
        StatusCode::FORBIDDEN,
        "selected list must only open for the owner's own close friends"
    );
    let (s, _) = req(
        &pool,
        "PUT",
        &format!("/me/close-friends/{}", b.id),
        &a.token,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert_eq!(status(b.token.clone()).await, StatusCode::OK);
    assert_eq!(
        status(c.token.clone()).await,
        StatusCode::FORBIDDEN,
        "selected list must only open for the owner's own close friends"
    );
    assert!(wishly_api::close_friends::is_close(&pool, a.id, b.id)
        .await
        .unwrap());
    assert!(!wishly_api::close_friends::is_close(&pool, b.id, a.id)
        .await
        .unwrap());
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn unfriend_removes_close_both_ways(pool: PgPool) {
    let a = user(&pool, "A").await;
    let b = user(&pool, "B").await;
    befriend(&pool, a.id, b.id).await;
    close(&pool, a.id, b.id).await;
    close(&pool, b.id, a.id).await;
    let (s, _) = req(
        &pool,
        "DELETE",
        &format!("/friends/{}", a.id),
        &b.token,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert_eq!(
        n_close(&pool).await,
        0,
        "unfriend must clear close_friends both ways"
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn block_removes_close_both_ways(pool: PgPool) {
    let a = user(&pool, "A").await;
    let b = user(&pool, "B").await;
    befriend(&pool, a.id, b.id).await;
    close(&pool, a.id, b.id).await;
    close(&pool, b.id, a.id).await;
    let (s, _) = req(
        &pool,
        "POST",
        &format!("/users/{}/block", a.id),
        &b.token,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert_eq!(n_close(&pool).await, 0);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn invite_accept_on_selected_adds_close(pool: PgPool) {
    let a = user(&pool, "A").await;
    let c = user(&pool, "C").await;
    let w = wishlist(
        &pool,
        a.id,
        ListOpts {
            visibility: Some("selected"),
            ..Default::default()
        },
    )
    .await;
    let (s, inv) = req(
        &pool,
        "POST",
        "/friends/invites",
        &a.token,
        Some(json!({ "wishlist_id": w.id })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{inv}");
    let t = inv["token"].as_str().unwrap();
    let (s, v) = req(
        &pool,
        "POST",
        &format!("/friends/invites/{t}/accept"),
        &c.token,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(wishly_api::close_friends::is_close(&pool, a.id, c.id)
        .await
        .unwrap());
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn allowed_users_api_removed(pool: PgPool) {
    let a = user(&pool, "A").await;
    let w = wishlist(&pool, a.id, ListOpts::default()).await;
    let (s, _) = req(
        &pool,
        "GET",
        &format!("/wishlists/{}/allowed-users", w.id),
        &a.token,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn limit_200(pool: PgPool) {
    let a = user(&pool, "A").await;
    let b = user(&pool, "B").await;
    befriend(&pool, a.id, b.id).await;
    sqlx::query("INSERT INTO users (display_name) SELECT 'x' || g FROM generate_series(1, 200) g")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO close_friends (owner_id, friend_id) SELECT $1, id FROM users WHERE display_name LIKE 'x%'")
        .bind(a.id)
        .execute(&pool)
        .await
        .unwrap();
    let u = format!("/me/close-friends/{}", b.id);
    let (s, v) = req(&pool, "PUT", &u, &a.token, None).await;
    assert_eq!(
        (s, v["code"].as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, Some("LIMIT"))
    );
    assert_eq!(n_close(&pool).await, 200);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn concurrent_adds_respect_limit(pool: PgPool) {
    let a = user(&pool, "A").await;
    let (b, c) = (user(&pool, "B").await, user(&pool, "C").await);
    befriend(&pool, a.id, b.id).await;
    befriend(&pool, a.id, c.id).await;
    sqlx::query("INSERT INTO users (display_name) SELECT 'x' || g FROM generate_series(1, 199) g")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO close_friends (owner_id, friend_id) SELECT $1, id FROM users WHERE display_name LIKE 'x%'")
        .bind(a.id)
        .execute(&pool)
        .await
        .unwrap();
    // 慢觸發器讓兩個不同好友的 INSERT 重疊；沒有 per-owner 鎖就都讀到 199 而寫成 201
    sqlx::raw_sql(
        "CREATE FUNCTION slow_cf() RETURNS trigger LANGUAGE plpgsql AS $$
         BEGIN PERFORM pg_sleep(0.3); RETURN NEW; END $$;
         CREATE TRIGGER slow_cf BEFORE INSERT ON close_friends FOR EACH ROW EXECUTE FUNCTION slow_cf()",
    )
    .execute(&pool)
    .await
    .unwrap();
    let app = common::test_app(&pool);
    let put = |f: Uuid| {
        let h = [ck(&a.token), idem(Uuid::new_v4())].concat();
        let app = app.clone();
        async move {
            common::call_app(&app, "PUT", &format!("/me/close-friends/{f}"), &h, None)
                .await
                .0
        }
    };
    let (r1, r2) = tokio::join!(put(b.id), put(c.id));
    let mut got = [r1, r2];
    got.sort();
    assert_eq!(
        got,
        [StatusCode::NO_CONTENT, StatusCode::UNPROCESSABLE_ENTITY],
        "concurrent adds exceeded the close-friends limit"
    );
    assert_eq!(n_close(&pool).await, 200);
}

async fn apply(pool: &PgPool, path: &std::path::Path) {
    sqlx::raw_sql(sqlx::AssertSqlSafe(std::fs::read_to_string(path).unwrap()))
        .execute(pool)
        .await
        .unwrap();
}

#[sqlx::test(migrations = false)]
async fn migration_0067_merges_and_privatizes(pool: PgPool) {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../db/migrations");
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    files.sort();
    let (old, rest): (Vec<_>, Vec<_>) = files
        .into_iter()
        .partition(|f| f.file_name().unwrap().to_str().unwrap() < "0067");
    for f in &old {
        apply(&pool, f).await;
    }
    let sel = || ListOpts {
        visibility: Some("selected"),
        ..Default::default()
    };
    let (o1, o2, o3) = (
        user(&pool, "o1").await,
        user(&pool, "o2").await,
        user(&pool, "o3").await,
    );
    let (f1, f2, ex) = (
        user(&pool, "f1").await,
        user(&pool, "f2").await,
        user(&pool, "ex").await,
    );
    let (kept, empty, gone, m1, m2) = (
        wishlist(&pool, o1.id, sel()).await,
        wishlist(&pool, o2.id, sel()).await,
        wishlist(&pool, o3.id, sel()).await,
        wishlist(&pool, o1.id, sel()).await,
        wishlist(&pool, o1.id, sel()).await,
    );
    for f in [f1.id, f2.id] {
        befriend(&pool, o1.id, f).await;
    }
    // kept 勾 f1；m1 勾 f1；m2 勾 f2（同一擁有者兩份清單勾不同人）；gone 勾已非好友的 ex；empty 無人
    for (w, u) in [
        (kept.id, f1.id),
        (m1.id, f1.id),
        (m2.id, f2.id),
        (gone.id, ex.id),
    ] {
        sqlx::query("INSERT INTO wishlist_allowed_users (wishlist_id, user_id) VALUES ($1, $2)")
            .bind(w)
            .bind(u)
            .execute(&pool)
            .await
            .unwrap();
    }
    for f in &rest {
        apply(&pool, f).await;
    }
    let rows: Vec<(Uuid, Uuid)> =
        sqlx::query_as("SELECT owner_id, friend_id FROM close_friends ORDER BY 1, 2")
            .fetch_all(&pool)
            .await
            .unwrap();
    let mut want = vec![(o1.id, f1.id), (o1.id, f2.id)];
    want.sort();
    assert_eq!(rows, want);
    let vis = |w: Uuid| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, String>("SELECT visibility::text FROM wishlists WHERE id=$1")
                .bind(w)
                .fetch_one(&pool)
                .await
                .unwrap()
        }
    };
    for w in [kept.id, m1.id, m2.id] {
        assert_eq!(vis(w).await, "selected");
    }
    for w in [empty.id, gone.id] {
        assert_eq!(vis(w).await, "private");
    }
}
