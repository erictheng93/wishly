//! 決策：訪客只能瀏覽，新認捐需登入；舊訪客捐助仍可管理（減量 / 備註 / 取消），但不可加量。
mod common;
use axum::http::StatusCode;
use common::*;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

/// (claims, guests, idempotency_keys, notifications)
async fn counts(pool: &PgPool) -> (i64, i64, i64, i64) {
    sqlx::query_as(
        "SELECT (SELECT count(*) FROM claims), (SELECT count(*) FROM guests),
                (SELECT count(*) FROM idempotency_keys), (SELECT count(*) FROM notifications)",
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn qty_claimed(pool: &PgPool, item: Uuid) -> i32 {
    sqlx::query_scalar("SELECT qty_claimed FROM wishlist_items WHERE id = $1")
        .bind(item)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn fixture(pool: &PgPool, needed: i32) -> (Uuid, Uuid) {
    let owner = user(pool, "owner").await;
    let wl = wishlist(pool, owner.id, ListOpts::default()).await;
    (wl.id, item(pool, wl.id, "奶瓶", needed).await)
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn anonymous_post_is_401_and_writes_nothing(pool: PgPool) {
    let (_, it) = fixture(&pool, 5).await;
    // 帶舊版訪客欄位也一樣：欄位被忽略，身分檢查先擋
    for body in [
        json!({ "qty": 1 }),
        json!({ "qty": 1, "display_name": "小明", "email": "a@example.com", "contact": "line:x" }),
    ] {
        let (s, _, b) = call(
            &pool,
            "POST",
            &format!("/items/{it}/claims"),
            &idem(Uuid::new_v4()),
            Some(body),
        )
        .await;
        assert_eq!(
            (s, b["code"].as_str()),
            (StatusCode::UNAUTHORIZED, Some("UNAUTHORIZED")),
            "{b}"
        );
    }
    assert_eq!(counts(&pool).await, (0, 0, 0, 0));
    assert_eq!(qty_claimed(&pool, it).await, 0);
    assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn legacy_guest_token_without_session_is_401_and_writes_nothing(pool: PgPool) {
    let (wl, held) = fixture(&pool, 5).await;
    let other = item(&pool, wl, "推車", 2).await;
    let (tok, _) = legacy_guest_claim(&pool, held, 2, None).await;
    let before = counts(&pool).await;
    assert_eq!(before, (1, 1, 0, 0));
    // header 與 ws_guest cookie 兩種帶法
    for h in [gt(&tok), vec![("cookie", format!("ws_guest={tok}"))]] {
        for target in [other, held] {
            let (s, _, b) = call(
                &pool,
                "POST",
                &format!("/items/{target}/claims"),
                &[h.clone(), idem(Uuid::new_v4())].concat(),
                Some(json!({ "qty": 1 })),
            )
            .await;
            assert_eq!(
                (s, b["code"].as_str()),
                (StatusCode::UNAUTHORIZED, Some("UNAUTHORIZED")),
                "{b}"
            );
        }
    }
    assert_eq!(counts(&pool).await, before);
    assert_eq!(
        (
            qty_claimed(&pool, held).await,
            qty_claimed(&pool, other).await
        ),
        (2, 0)
    );
    assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn signed_in_claim_uses_account_display_name(pool: PgPool) {
    let (_, it) = fixture(&pool, 5).await;
    let me = user(&pool, "會員小明").await;
    // 舊版欄位被忽略（CreateReq 不拒絕未知欄位）：名稱一律取帳號暱稱，不建 guest
    let (s, h, b) = call(
        &pool,
        "POST",
        &format!("/items/{it}/claims"),
        &[ck(&me.token), idem(Uuid::new_v4())].concat(),
        Some(json!({ "qty": 2, "display_name": "冒名", "email": "x@example.com", "contact": "c" })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    assert_eq!(b["claim"]["claimer_name"], "會員小明");
    assert!(b.get("guest_token").is_none() && !h.contains_key("set-cookie"));
    assert_eq!(b["item"]["qty_claimed"], 2);
    let (c, g, ..) = counts(&pool).await;
    assert_eq!((c, g), (1, 0));
    let uid: Option<Uuid> = sqlx::query_scalar("SELECT user_id FROM claims")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(uid, Some(me.id));
    // 會員本人仍可加量
    let cid = b["claim"]["id"].as_str().unwrap();
    let (s, _, b) = call(
        &pool,
        "PATCH",
        &format!("/claims/{cid}"),
        &ck(&me.token),
        Some(json!({ "qty": 4 })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(qty_claimed(&pool, it).await, 4);
    assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn legacy_guest_can_cancel_and_decrease_but_not_increase(pool: PgPool) {
    let (_, it) = fixture(&pool, 10).await;
    let (tok, cid) = legacy_guest_claim(&pool, it, 3, None).await;
    let url = format!("/claims/{cid}");
    let patch = |body: serde_json::Value| {
        let (pool, url, tok) = (pool.clone(), url.clone(), tok.clone());
        async move { call(&pool, "PATCH", &url, &gt(&tok), Some(body)).await }
    };
    // 加量 → 401，qty_claimed 不變
    let (s, _, b) = patch(json!({ "qty": 5 })).await;
    assert_eq!(
        (s, b["code"].as_str()),
        (StatusCode::UNAUTHORIZED, Some("UNAUTHORIZED")),
        "{b}"
    );
    assert_eq!(qty_claimed(&pool, it).await, 3);
    // 減量
    let (s, _, b) = patch(json!({ "qty": 2 })).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(qty_claimed(&pool, it).await, 2);
    // 備註（不動數量）
    let (s, b) = {
        let r = patch(json!({ "note": "週末到貨" })).await;
        (r.0, r.2)
    };
    assert_eq!(
        (s, b["claim"]["note"].as_str()),
        (StatusCode::OK, Some("週末到貨"))
    );
    assert_eq!(qty_claimed(&pool, it).await, 2);
    // 減量後再加回原值也是加量 → 401
    assert_eq!(patch(json!({ "qty": 3 })).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(qty_claimed(&pool, it).await, 2);
    // 取消回補
    let (s, ..) = call(&pool, "DELETE", &url, &gt(&tok), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert_eq!(qty_claimed(&pool, it).await, 0);
    assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn guest_me_still_lists_legacy_claim(pool: PgPool) {
    let (_, it) = fixture(&pool, 5).await;
    let (tok, cid) = legacy_guest_claim(&pool, it, 2, None).await;
    let (s, h, me) = call(&pool, "GET", "/guest/me", &gt(&tok), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(h["cache-control"], "private, no-store");
    assert_eq!(me["claims"].as_array().map(Vec::len), Some(1));
    assert_eq!(me["claims"][0]["claim"]["id"], cid.to_string());
    assert_eq!(me["claims"][0]["item"]["title"], "奶瓶");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn signed_in_member_can_manage_own_legacy_guest_claim(pool: PgPool) {
    let (_, it) = fixture(&pool, 10).await;
    let (tok, cid) = legacy_guest_claim(&pool, it, 3, None).await;
    let me = user(&pool, "former-guest").await;
    let url = format!("/claims/{cid}");
    let both = || [ck(&me.token), gt(&tok)].concat();
    // 只帶會員 session、沒帶舊訪客 token → 不是本人
    let (s, _, b) = call(
        &pool,
        "PATCH",
        &url,
        &ck(&me.token),
        Some(json!({ "qty": 2 })),
    )
    .await;
    assert_eq!(
        (s, b["code"].as_str()),
        (StatusCode::NOT_FOUND, Some("NOT_FOUND")),
        "{b}"
    );
    assert_eq!(qty_claimed(&pool, it).await, 3);
    // 會員 + 舊訪客 token：加量 → 403 LEGACY_CLAIM_READONLY，數量不變
    let (s, _, b) = call(&pool, "PATCH", &url, &both(), Some(json!({ "qty": 5 }))).await;
    assert_eq!(
        (s, b["code"].as_str()),
        (StatusCode::FORBIDDEN, Some("LEGACY_CLAIM_READONLY")),
        "{b}"
    );
    assert_eq!(qty_claimed(&pool, it).await, 3);
    // 減量、取消可以
    let (s, _, b) = call(&pool, "PATCH", &url, &both(), Some(json!({ "qty": 1 }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(qty_claimed(&pool, it).await, 1);
    let (s, ..) = call(&pool, "DELETE", &url, &both(), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert_eq!(qty_claimed(&pool, it).await, 0);
    assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn member_with_stale_guest_token_still_manages_own_claims(pool: PgPool) {
    // 會員自己的捐助不受殘留的失效訪客 token 影響
    let (_, it) = fixture(&pool, 10).await;
    let (tok, cid) = user_claim(&pool, it, 2).await;
    let hdr = [ck(&tok), gt("stale-token")].concat();
    let (s, _, b) = call(
        &pool,
        "PATCH",
        &format!("/claims/{cid}"),
        &hdr,
        Some(json!({ "qty": 4 })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(qty_claimed(&pool, it).await, 4);
}

/// PATCH 加量等同新認捐：清單截止 / 下架 / 轉私人 / 被解除好友後都不可再加量；減量仍可。
#[sqlx::test(migrations = "../../db/migrations")]
async fn qty_increase_reapplies_create_gates(pool: PgPool) {
    let cases = [
        (
            "deadline",
            "UPDATE wishlists SET closes_at = now() - interval '1 hour' WHERE id = $1",
        ),
        (
            "hidden",
            "UPDATE wishlists SET moderation_status = 'hidden', moderation_reason = 'x' WHERE id = $1",
        ),
        (
            "private",
            "UPDATE wishlists SET visibility = 'private' WHERE id = $1",
        ),
        (
            "unfriended",
            "DELETE FROM friendships WHERE $1::uuid IS NOT NULL",
        ),
    ];
    for (name, sql) in cases {
        let owner = user(&pool, "owner").await;
        let vis = if name == "unfriended" {
            "friends"
        } else {
            "public"
        };
        let wl = wishlist(
            &pool,
            owner.id,
            ListOpts {
                visibility: Some(vis),
                closes_in: Some("1 day"),
                ..Default::default()
            },
        )
        .await;
        let it = item(&pool, wl.id, "奶瓶", 9).await;
        let u = user(&pool, "u").await;
        if name == "unfriended" {
            befriend(&pool, owner.id, u.id).await;
        }
        let (s, _, b) = call(
            &pool,
            "POST",
            &format!("/items/{it}/claims"),
            &[ck(&u.token), idem(Uuid::new_v4())].concat(),
            Some(json!({ "qty": 2 })),
        )
        .await;
        assert_eq!(s, StatusCode::CREATED, "{name} {b}");
        let cid = b["claim"]["id"].as_str().unwrap().to_string();
        sqlx::query(sql).bind(wl.id).execute(&pool).await.unwrap();
        let (s, _, b) = call(
            &pool,
            "PATCH",
            &format!("/claims/{cid}"),
            &ck(&u.token),
            Some(json!({ "qty": 4 })),
        )
        .await;
        assert!(s.is_client_error(), "{name} 加量應被拒 {s} {b}");
        assert_eq!(qty_claimed(&pool, it).await, 2, "{name}");
        let (s, _, b) = call(
            &pool,
            "PATCH",
            &format!("/claims/{cid}"),
            &ck(&u.token),
            Some(json!({ "qty": 1 })),
        )
        .await;
        assert_eq!(s, StatusCode::OK, "{name} 減量仍可 {b}");
        assert_eq!(qty_claimed(&pool, it).await, 1, "{name}");
    }
}
