//! Q13：建立者修改品項內容時，提醒進行中的捐助者（合併 email + 我的捐助頁提示）。
mod common;
use axum::http::StatusCode;
use common::*;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

async fn pending(pool: &PgPool, item: Uuid) -> Vec<(Option<Uuid>, Option<Uuid>, bool)> {
    sqlx::query_as(
        "SELECT user_id, guest_id, scheduled_at > now() + interval '9 minutes' FROM notifications
          WHERE kind = 'claim.item_changed' AND payload->>'item_id' = $1 AND status = 'pending' ORDER BY user_id, guest_id",
    )
    .bind(item.to_string())
    .fetch_all(pool)
    .await
    .unwrap()
}

async fn patch(pool: &PgPool, owner: &str, item: Uuid, body: Value) {
    let (s, _, b) = call(
        pool,
        "PATCH",
        &format!("/items/{item}"),
        &ck(owner),
        Some(body),
    )
    .await;
    // 不印 body：500 的 INTERNAL_ERROR 會被 mutate.py 當成基礎設施失敗
    assert_eq!(
        s,
        StatusCode::OK,
        "item PATCH failed: {}",
        b["code"] == "INTERNAL_ERROR"
    );
}

async fn me_row(pool: &PgPool, token: &str, claim: Uuid) -> Value {
    let (s, _, b) = call(pool, "GET", "/guest/me", &ck(token), None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    b["claims"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["claim"]["id"] == claim.to_string())
        .cloned()
        .unwrap()
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn edits_coalesce_into_one_pending_mail_per_active_donor(pool: PgPool) {
    let owner = user(&pool, "擁有者").await;
    let wl = wishlist(&pool, owner.id, ListOpts::default()).await.id;
    let it = item(&pool, wl, "玻璃奶瓶", 10).await;
    let (_, _) = user_claim(&pool, it, 1).await;
    let (tok_b, claim_b) = user_claim(&pool, it, 1).await;
    let (_, _) = legacy_guest_claim(&pool, it, 1, Some("g@example.com")).await;
    let (_, _) = legacy_guest_claim(&pool, it, 1, None).await; // 無 email：不寄
                                                               // 已取消 / 逾期的捐助者不通知
    let (_, gone) = user_claim(&pool, it, 1).await;
    let (_, expired) = user_claim(&pool, it, 1).await;
    sqlx::query("UPDATE claims SET status = CASE WHEN id = $1 THEN 'cancelled'::claim_status ELSE 'expired'::claim_status END WHERE id = ANY($2)")
        .bind(gone).bind(vec![gone, expired]).execute(&pool).await.unwrap();
    sqlx::query("UPDATE wishlist_items SET qty_claimed = qty_claimed - 2 WHERE id = $1")
        .bind(it)
        .execute(&pool)
        .await
        .unwrap();

    patch(&pool, &owner.token, it, json!({ "qty_needed": 8 })).await;
    patch(&pool, &owner.token, it, json!({ "spec": "240ml" })).await;
    patch(
        &pool,
        &owner.token,
        it,
        json!({ "title": "玻璃奶瓶 240ml" }),
    )
    .await;
    let rows = pending(&pool, it).await;
    assert_eq!(
        rows.len(),
        3,
        "one pending item_changed per donor: {rows:?}"
    ); // 兩位會員 + 有 email 的訪客，各一筆
    assert!(rows.iter().all(|r| !r.2), "站內要立刻出現：排程不延後");
    let changed: Option<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar("SELECT content_changed_at FROM wishlist_items WHERE id = $1")
            .bind(it)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(changed.is_some());
    assert!(me_row(&pool, &tok_b, claim_b).await["item_changed_at"].is_string());
    assert_claims_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn same_values_and_reorder_do_not_notify(pool: PgPool) {
    let owner = user(&pool, "擁有者").await;
    let wl = wishlist(&pool, owner.id, ListOpts::default()).await.id;
    let it = item(&pool, wl, "玻璃奶瓶", 3).await;
    let other = item(&pool, wl, "口水巾", 1).await;
    let (tok, claim) = user_claim(&pool, it, 1).await;
    patch(
        &pool,
        &owner.token,
        it,
        json!({ "title": "玻璃奶瓶", "qty_needed": 3 }),
    )
    .await;
    let (s, _, b) = call(
        &pool,
        "POST",
        &format!("/wishlists/{wl}/items/reorder"),
        &ck(&owner.token),
        Some(json!({ "item_ids": [other, it] })),
    )
    .await;
    assert!(s.is_success(), "{b}");
    assert!(pending(&pool, it).await.is_empty());
    let changed: Option<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar("SELECT content_changed_at FROM wishlist_items WHERE id = $1")
            .bind(it)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(changed.is_none());
    assert!(me_row(&pool, &tok, claim).await["item_changed_at"].is_null());
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn ack_clears_notice_until_next_edit(pool: PgPool) {
    let owner = user(&pool, "擁有者").await;
    let wl = wishlist(&pool, owner.id, ListOpts::default()).await.id;
    let it = item(&pool, wl, "玻璃奶瓶", 3).await;
    let (tok, claim) = user_claim(&pool, it, 1).await;
    patch(&pool, &owner.token, it, json!({ "qty_needed": 2 })).await;
    assert!(me_row(&pool, &tok, claim).await["item_changed_at"].is_string());

    // 別人（含建立者以外的會員）確認：一律 404，不洩漏捐助存在
    let stranger = user(&pool, "路人").await;
    let ack = json!({ "ack_item_change": true });
    let path = format!("/claims/{claim}");
    let (s, _, _) = call(
        &pool,
        "PATCH",
        &path,
        &ck(&stranger.token),
        Some(ack.clone()),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    let audits = || async {
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM audit_logs WHERE entity_id = $1")
            .bind(claim)
            .fetch_one(&pool)
            .await
            .unwrap()
    };
    let before = audits().await;
    let (s, _, b) = call(&pool, "PATCH", &path, &ck(&tok), Some(ack)).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(
        (b["claim"]["status"].as_str(), b["claim"]["qty"].as_i64()),
        (Some("reserved"), Some(1))
    );
    assert_eq!(audits().await, before, "確認不算狀態 / 數量變更");
    assert!(me_row(&pool, &tok, claim).await["item_changed_at"].is_null());

    patch(
        &pool,
        &owner.token,
        it,
        json!({ "description": "要玻璃材質" }),
    )
    .await;
    assert!(me_row(&pool, &tok, claim).await["item_changed_at"].is_string());
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn crowdfund_pledgers_are_notified(pool: PgPool) {
    let owner = user(&pool, "擁有者").await;
    let wl = wishlist(&pool, owner.id, ListOpts::default()).await.id;
    let it = crowdfund_item(&pool, wl, 1000).await;
    let fan = user(&pool, "支持者").await;
    grant(&pool, fan.id, 500).await;
    let (s, _, b) = pledge(&test_app(&pool), it, &fan.token, 100).await;
    assert!(s.is_success(), "{b}");
    patch(
        &pool,
        &owner.token,
        it,
        json!({ "description": "雙向推車" }),
    )
    .await;
    assert_eq!(pending(&pool, it).await, vec![(Some(fan.id), None, false)]);
    let crowdfund: bool = sqlx::query_scalar(
        "SELECT (payload->>'crowdfund')::bool FROM notifications WHERE kind = 'claim.item_changed'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(crowdfund);
}

#[test]
fn renders_item_title_in_all_languages() {
    use wishly_api::notify::{render_lang, Lang};
    let p = json!({ "item_title": "玻璃奶瓶" });
    for (l, word) in [
        (Lang::Zh, "確認"),
        (Lang::En, "re-check"),
        (Lang::Vi, "kiểm tra lại"),
        (Lang::Id, "cek lagi"),
    ] {
        let (s, b) = render_lang(l, "claim.item_changed", "寶寶清單", &p);
        assert!(s.contains("玻璃奶瓶") && b.contains("玻璃奶瓶"), "{l:?}");
        assert!(
            b.contains("寶寶清單") && b.contains(word) && b.contains("/me/claims"),
            "{b}"
        );
    }
    let (_, b) = render_lang(
        Lang::Zh,
        "claim.item_changed",
        "x",
        &json!({ "crowdfund": true }),
    );
    assert!(b.contains("/me/wallet"));
}

async fn notif_status(
    pool: &PgPool,
    item: Uuid,
    user_id: Option<Uuid>,
) -> (String, Option<String>) {
    sqlx::query_as("SELECT status::text, last_error FROM notifications WHERE kind = 'claim.item_changed' AND payload->>'item_id' = $1 AND user_id IS NOT DISTINCT FROM $2")
        .bind(item.to_string()).bind(user_id).fetch_one(pool).await.unwrap()
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn cancelled_claimant_notice_is_cancelled_at_send_time(pool: PgPool) {
    let owner = user(&pool, "擁有者").await;
    let wl = wishlist(&pool, owner.id, ListOpts::default()).await.id;
    let it = item(&pool, wl, "玻璃奶瓶", 5).await;
    let (_, gone) = user_claim(&pool, it, 1).await;
    let (_, _active) = user_claim(&pool, it, 1).await;
    patch(&pool, &owner.token, it, json!({ "description": "改了" })).await;
    let gone_uid: Uuid = sqlx::query_scalar("SELECT user_id FROM claims WHERE id = $1")
        .bind(gone)
        .fetch_one(&pool)
        .await
        .unwrap();
    let active_uid: Uuid =
        sqlx::query_scalar("SELECT user_id FROM claims WHERE id <> $1 AND item_id = $2")
            .bind(gone)
            .bind(it)
            .fetch_one(&pool)
            .await
            .unwrap();
    sqlx::query("UPDATE claims SET status = 'cancelled' WHERE id = $1")
        .bind(gone)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE notifications SET created_at = now() - interval '11 minutes', scheduled_at = now() WHERE kind = 'claim.item_changed'")
        .execute(&pool)
        .await
        .unwrap();
    wishly_api::notify::tick_at(&pool, common::noon()).await.unwrap();
    assert_eq!(
        notif_status(&pool, it, Some(gone_uid)).await,
        ("cancelled".into(), Some("no longer supporting item".into()))
    );
    assert_eq!(notif_status(&pool, it, Some(active_uid)).await.0, "sent");
}
