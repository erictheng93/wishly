//! 探索頁 GET /public/wishlists：只列公開、上線、有品項、未驚喜鎖定的清單；訪客可讀、可被 CDN 快取
mod common;
use axum::http::StatusCode;
use common::*;
use sqlx::PgPool;

async fn titles(pool: &PgPool, q: &str) -> (Vec<String>, serde_json::Value) {
    let (s, h, v) = call(pool, "GET", &format!("/public/wishlists{q}"), &[], None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(h["cache-control"].to_str().unwrap().starts_with("public"));
    let t = v["wishlists"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["title"].as_str().unwrap().to_string())
        .collect();
    (t, v)
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn explore_lists_only_servable_public_lists(pool: PgPool) {
    let o = user(&pool, "主人").await;
    let cases: [(&str, ListOpts, bool); 7] = [
        ("公開", ListOpts::default(), true),
        (
            "好友",
            ListOpts {
                visibility: Some("friends"),
                ..Default::default()
            },
            false,
        ),
        (
            "摯友",
            ListOpts {
                visibility: Some("selected"),
                ..Default::default()
            },
            false,
        ),
        (
            "私人",
            ListOpts {
                visibility: Some("private"),
                ..Default::default()
            },
            false,
        ),
        (
            "草稿",
            ListOpts {
                status: Some("draft"),
                ..Default::default()
            },
            false,
        ),
        (
            "下架",
            ListOpts {
                moderation_status: Some("hidden"),
                ..Default::default()
            },
            false,
        ),
        (
            "驚喜",
            ListOpts {
                surprise: true,
                event_offset_days: Some(30),
                ..Default::default()
            },
            false,
        ),
    ];
    for (title, opts, _) in cases {
        let l = wishlist(&pool, o.id, opts).await;
        sqlx::query("UPDATE wishlists SET title=$2 WHERE id=$1")
            .bind(l.id)
            .bind(title)
            .execute(&pool)
            .await
            .unwrap();
        item(&pool, l.id, "奶瓶", 1).await;
    }
    let empty = wishlist(&pool, o.id, ListOpts::default()).await; // 沒有品項
    sqlx::query("UPDATE wishlists SET title='空清單' WHERE id=$1")
        .bind(empty.id)
        .execute(&pool)
        .await
        .unwrap();

    let (t, v) = titles(&pool, "").await;
    assert_eq!(t, vec!["公開"], "{v}");
    assert_eq!(v["wishlists"][0]["owner"]["display_name"], "主人");
    assert!(v["next_page"].is_null());
    // 型別篩選：fixture 為 registry
    assert_eq!(titles(&pool, "?type=registry").await.0, vec!["公開"]);
    assert!(titles(&pool, "?type=relief").await.0.is_empty());
    let (s, _, _) = call(&pool, "GET", "/public/wishlists?type=nope", &[], None).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    // 擁有者被封鎖 → 不列
    sqlx::query("UPDATE users SET blocked_at = now() WHERE id=$1")
        .bind(o.id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(titles(&pool, "").await.0.is_empty());
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn explore_paginates(pool: PgPool) {
    let o = user(&pool, "主人").await;
    for _ in 0..25 {
        let l = wishlist(&pool, o.id, ListOpts::default()).await;
        item(&pool, l.id, "奶瓶", 1).await;
    }
    let (p1, v) = titles(&pool, "").await;
    assert_eq!((p1.len(), v["next_page"].as_i64()), (24, Some(2)));
    let (p2, v) = titles(&pool, "?page=2").await;
    assert_eq!((p2.len(), v["next_page"].is_null()), (1, true));
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn demo_seed_hidden_from_explore(pool: PgPool) {
    for _ in 0..2 {
        // 第二次驗證可重複執行
        sqlx::raw_sql(include_str!("../../../db/seed/demo.sql"))
            .execute(&pool)
            .await
            .unwrap();
    }
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM wishlists WHERE slug LIKE 'demo%' AND visibility = 'public'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(n, 3);
    assert!(titles(&pool, "").await.0.is_empty());
}
