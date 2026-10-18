mod common;
use axum::http::StatusCode;
use sqlx::PgPool;
use uuid::Uuid;

/// 回傳 (user_id, session token)
use common::*;

#[sqlx::test(migrations = "../../db/migrations")]
async fn surprise_dashboard_hides_claimers(pool: PgPool) {
    let TestUser {
        id: owner,
        token: tok,
    } = user(&pool, "owner").await;
    let TestUser {
        id: stranger,
        token: stok,
    } = user(&pool, "x").await;
    let _ = stranger;
    let (wid, _, _) = claimed_wishlist(&pool, owner, true, 30, 2, 1).await;
    let (s, _, v) = call(
        &pool,
        "GET",
        &format!("/wishlists/{wid}/dashboard"),
        &ck(&tok),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let raw = v.to_string();
    assert!(
        !raw.contains("秘密客") && !raw.contains("line:secret"),
        "leak: {raw}"
    );
    assert_eq!(v["surprise_locked"], true);
    assert!(v["claims"].is_null() && v["items"][0]["qty_claimed"].is_null());
    assert_eq!(v["totals"]["completion_pct"], 50); // 數量計 1/2（totals.qty_claimed 本就輸出）
                                                   // 非擁有者 → 404
    assert_eq!(
        call(
            &pool,
            "GET",
            &format!("/wishlists/{wid}/dashboard"),
            &ck(&stok),
            None
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(
            &pool,
            "GET",
            &format!("/wishlists/{wid}/dashboard"),
            &[],
            None
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    // 活動日已過 → 解鎖，看得到捐助者
    sqlx::query("UPDATE wishlists SET event_date = current_date - 1 WHERE id = $1")
        .bind(wid)
        .execute(&pool)
        .await
        .unwrap();
    let (_, _, v) = call(
        &pool,
        "GET",
        &format!("/wishlists/{wid}/dashboard"),
        &ck(&tok),
        None,
    )
    .await;
    assert_eq!(v["surprise_locked"], false);
    assert_eq!(v["claims"].as_array().map(|a| a.len()), Some(1));
    assert_eq!(v["items"][0]["qty_claimed"], 1);
    assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn unified_pct_owner_names_list_fields_and_image_base(pool: PgPool) {
    let TestUser {
        id: owner,
        token: tok,
    } = user(&pool, "owner").await;
    let (wid, slug, _) = claimed_wishlist(&pool, owner, false, -1, 2, 1).await; // 1/2 claimed, show_claimer_names 預設 false
                                                                                // 第二品項 1/3（數量計 2/5=40；以品項計會是 0 或 50）；soft-deleted 品項不計
    let second: Uuid = sqlx::query_scalar("INSERT INTO wishlist_items (wishlist_id, title, qty_needed, qty_claimed) VALUES ($1, 'b', 3, 1) RETURNING id").bind(wid).fetch_one(&pool).await.unwrap();
    sqlx::query("INSERT INTO claims (item_id, guest_id, claimer_name, qty) SELECT $1, guest_id, claimer_name, 1 FROM claims WHERE item_id IN (SELECT id FROM wishlist_items WHERE wishlist_id=$2) LIMIT 1")
        .bind(second).bind(wid).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO wishlist_items (wishlist_id, title, qty_needed, qty_claimed, deleted_at) VALUES ($1, 'gone', 100, 0, now())").bind(wid).execute(&pool).await.unwrap();
    sqlx::query("UPDATE wishlists SET cover_image_key = 'covers/x.jpg', cover_image_status = 'ready', moderation_status = 'ok' WHERE id = $1").bind(wid).execute(&pool).await.unwrap();

    let (_, _, d) = call(
        &pool,
        "GET",
        &format!("/wishlists/{wid}/dashboard"),
        &ck(&tok),
        None,
    )
    .await;
    assert_eq!(d["claims"][0]["claimer_name"], "秘密客");
    assert_eq!(d["totals"]["completion_pct"], 40);

    let (_, _, l) = call(&pool, "GET", "/wishlists", &ck(&tok), None).await;
    assert_eq!(l["data"][0]["completion"]["completion_pct"], 40);
    assert_eq!(l["data"][0]["moderation_status"], "ok");
    assert!(l["data"][0].get("moderation_reason").is_some());

    let (_, _, p) = call(
        &pool,
        "GET",
        &format!("/public/wishlists/{slug}"),
        &[],
        None,
    )
    .await;
    assert_eq!(p["completion"]["completion_pct"], 40);
    assert_eq!(
        p["cover_image_url"],
        wishly_api::uploads::S3::get().public_url("covers/x.jpg")
    );
    assert_eq!(wishly_api::wishlists::completion_pct(0, 0), 0);
    assert_all_reconciled(&pool).await;
}

// SSE 連線建立後失去存取權（取消好友、可見性改 private）：串流須關閉，不再推送標題/數量。
// dashboard::BUS 為 process-wide，此檔僅此一個 SSE 測試（見 wishlist_events.rs 註解）。
#[sqlx::test(migrations = "../../db/migrations")]
async fn sse_closes_when_viewer_loses_access(pool: PgPool) {
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let owner = user(&pool, "o").await;
    let fan = user(&pool, "f").await;
    befriend(&pool, owner.id, fan.id).await;
    let l = wishlist(
        &pool,
        owner.id,
        ListOpts {
            visibility: Some("friends"),
            ..Default::default()
        },
    )
    .await;
    item(&pool, l.id, "禮物", 1).await;
    let open = || async {
        let req = Request::builder()
            .uri(format!("/api/v1/public/wishlists/{}/events", l.slug))
            .header("cookie", format!("ws_session={}", fan.token))
            .body(Body::empty())
            .unwrap();
        let res = test_app(&pool).oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        res.into_body()
    };
    // 串流是否在失去存取權後結束，且期間沒有再推 wishlist.updated
    async fn ends(mut body: Body) -> bool {
        let mut got = String::new();
        loop {
            match tokio::time::timeout(std::time::Duration::from_secs(5), body.frame()).await {
                Err(_) => return false,
                Ok(None) => return !got.contains("wishlist.updated"),
                Ok(Some(f)) => {
                    if let Some(d) = f.unwrap().data_ref() {
                        got.push_str(&String::from_utf8_lossy(d));
                    }
                }
            }
        }
    }
    // 1) 取消好友後有任何事件 → 關閉
    let body = open().await;
    sqlx::query("DELETE FROM friendships")
        .execute(&pool)
        .await
        .unwrap();
    wishly_api::dashboard::notify(&pool, l.id).await.unwrap();
    assert!(ends(body).await, "unfriend 後串流仍在推送");
    // 2) 可見性改 private → 關閉
    befriend(&pool, owner.id, fan.id).await;
    let body = open().await;
    sqlx::query("UPDATE wishlists SET visibility='private' WHERE id=$1")
        .bind(l.id)
        .execute(&pool)
        .await
        .unwrap();
    wishly_api::dashboard::notify(&pool, l.id).await.unwrap();
    assert!(ends(body).await, "改 private 後串流仍在推送");
    tokio::time::timeout(std::time::Duration::from_secs(10), pool.close())
        .await
        .expect("SSE listener must release its connection when the pool closes");
}
