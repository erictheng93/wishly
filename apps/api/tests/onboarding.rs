//! 新手導覽（D66）：步驟由資料推導、依序前進；完成 / 跳過 / 重新開始只動 onboarding_done_at。
mod common;
use axum::http::StatusCode;
use common::{call, ck, idem, item, user, wishlist, ListOpts};
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

fn steps(b: &Value) -> Vec<(String, bool)> {
    b["steps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| {
            (
                s["key"].as_str().unwrap().to_string(),
                s["done"].as_bool().unwrap(),
            )
        })
        .collect()
}

async fn current(pool: &PgPool, token: &str) -> Value {
    let (s, _, b) = call(pool, "GET", "/me/onboarding", &ck(token), None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    b
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn steps_follow_the_users_own_data(pool: PgPool) {
    let me = user(&pool, "新手").await;
    let b = current(&pool, &me.token).await;
    assert_eq!(b["done"], false);
    assert_eq!(b["current"], "profile");
    assert_eq!(
        steps(&b)
            .iter()
            .map(|(k, _)| k.as_str())
            .collect::<Vec<_>>(),
        ["profile", "wishlist", "item", "publish", "give"]
    );
    assert!(b["wishlist_id"].is_null());
    let (_, _, m) = call(&pool, "GET", "/me", &ck(&me.token), None).await;
    assert_eq!(m["onboarding_done"], false);

    // 1. 個人資料：填任一欄（代號 / 自介 / 生日）即算完成
    let (s, _, _) = call(
        &pool,
        "PATCH",
        "/me",
        &ck(&me.token),
        Some(json!({ "bio": "喜歡咖啡" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(current(&pool, &me.token).await["current"], "wishlist");

    // 2. 建立清單（草稿）→ 帶出清單 id 給「加品項」用
    let wl = wishlist(
        &pool,
        me.id,
        ListOpts {
            status: Some("draft"),
            ..Default::default()
        },
    )
    .await;
    let b = current(&pool, &me.token).await;
    assert_eq!(b["current"], "item");
    assert_eq!(b["wishlist_id"], wl.id.to_string());

    // 3. 品項：已刪除的不算
    let it = item(&pool, wl.id, "咖啡豆", 1).await;
    sqlx::query("UPDATE wishlist_items SET deleted_at = now() WHERE id = $1")
        .bind(it)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(current(&pool, &me.token).await["current"], "item");
    item(&pool, wl.id, "手沖壺", 1).await;
    assert_eq!(current(&pool, &me.token).await["current"], "publish");

    // 4. 發佈（成為受捐人）
    sqlx::query("UPDATE wishlists SET status = 'active' WHERE id = $1")
        .bind(wl.id)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(current(&pool, &me.token).await["current"], "give");

    // 5. 捐助別人的品項（成為捐助人）
    let friend = user(&pool, "朋友").await;
    let fl = wishlist(&pool, friend.id, ListOpts::default()).await;
    let gift = item(&pool, fl.id, "繪本", 1).await;
    let (s, _, b) = call(
        &pool,
        "POST",
        &format!("/items/{gift}/claims"),
        &[ck(&me.token), idem(Uuid::new_v4())].concat(),
        Some(json!({ "qty": 1 })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let b = current(&pool, &me.token).await;
    assert!(b["current"].is_null());
    assert!(steps(&b).iter().all(|(_, d)| *d));
    assert_eq!(b["done"], false, "全部走完也要使用者按「完成」才收起導覽");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn skip_and_restart(pool: PgPool) {
    let me = user(&pool, "懂的人").await;
    let (s, _, b) = call(
        &pool,
        "PATCH",
        "/me/onboarding",
        &ck(&me.token),
        Some(json!({ "done": true })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["done"], true);
    assert_eq!(b["current"], "profile", "跳過不會把步驟標成完成");
    let first: Option<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar("SELECT onboarding_done_at FROM users WHERE id = $1")
            .bind(me.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    // 再按一次不覆寫第一次的時間
    call(
        &pool,
        "PATCH",
        "/me/onboarding",
        &ck(&me.token),
        Some(json!({ "done": true })),
    )
    .await;
    let again: Option<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar("SELECT onboarding_done_at FROM users WHERE id = $1")
            .bind(me.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(first, again);
    let (_, _, m) = call(&pool, "GET", "/me", &ck(&me.token), None).await;
    assert_eq!(m["onboarding_done"], true);

    let (s, _, b) = call(
        &pool,
        "PATCH",
        "/me/onboarding",
        &ck(&me.token),
        Some(json!({ "done": false })),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(b["done"], false);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn org_sites_do_not_count_and_login_required(pool: PgPool) {
    let (s, _, _) = call(&pool, "GET", "/me/onboarding", &[], None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (s, _, _) = call(
        &pool,
        "PATCH",
        "/me/onboarding",
        &[],
        Some(json!({ "done": true })),
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);

    // 別人的清單不算我的
    let me = user(&pool, "甲").await;
    let other = user(&pool, "乙").await;
    wishlist(&pool, other.id, ListOpts::default()).await;
    let b = current(&pool, &me.token).await;
    assert_eq!(steps(&b)[1], ("wishlist".to_string(), false));
}
