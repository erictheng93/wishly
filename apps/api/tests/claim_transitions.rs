//! F5 捐助狀態機：目標狀態冪等、非法轉換、非本人 404（流程驗證 F4/F5）。
mod common;
use axum::http::StatusCode;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use common::*;

async fn stamps(pool: &PgPool, id: Uuid) -> (Option<String>, Option<String>) {
    sqlx::query_as("SELECT purchased_at::text, delivered_at::text FROM claims WHERE id=$1")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn same_status_patch_is_idempotent_and_keeps_timestamps(pool: PgPool) {
    let TestUser { id: o, .. } = user(&pool, "o").await;
    let wl = wishlist(&pool, o, ListOpts::default()).await;
    let it = item(&pool, wl.id, "奶瓶", 3).await;
    let (tok, c) = user_claim(&pool, it, 2).await;
    let url = format!("/claims/{c}");
    let h = ck(&tok);
    let patch = |b| call(&pool, "PATCH", &url, &h, Some(b));

    assert_eq!(patch(json!({"status":"purchased"})).await.0, StatusCode::OK);
    let first = stamps(&pool, c).await;
    // 連點 / 重送：同一目標狀態回 200 與目前資料，不重寫時間、不重複 audit
    let (s, _, b) = patch(json!({"status":"purchased"})).await;
    assert_eq!(
        (s, b["claim"]["status"].as_str()),
        (StatusCode::OK, Some("purchased")),
        "{b}"
    );
    // 只改備註也不可重設 purchased_at
    assert_eq!(patch(json!({"note":"已下單"})).await.0, StatusCode::OK);
    assert_eq!(stamps(&pool, c).await, first);

    assert_eq!(patch(json!({"status":"delivered"})).await.0, StatusCode::OK);
    let delivered = stamps(&pool, c).await;
    assert!(delivered.1.is_some());
    let (s, _, b) = patch(json!({"status":"delivered"})).await;
    assert_eq!(
        (s, b["item"]["qty_claimed"].as_i64()),
        (StatusCode::OK, Some(2)),
        "{b}"
    );
    assert_eq!(stamps(&pool, c).await, delivered);

    let audits: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_logs WHERE entity_id=$1 AND action='claim.update'",
    )
    .bind(c)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(audits, 2, "只有真正轉換才寫 audit");
    assert_claims_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn illegal_transitions_and_strangers(pool: PgPool) {
    let TestUser { id: o, .. } = user(&pool, "o").await;
    let TestUser { token: st, .. } = user(&pool, "路人").await;
    let wl = wishlist(&pool, o, ListOpts::default()).await;
    let it = item(&pool, wl.id, "奶瓶", 3).await;
    let (tok, c) = user_claim(&pool, it, 1).await;
    let url = format!("/claims/{c}");

    // 非本人、非擁有者：一律 404（不洩漏捐助存在）
    for (m, b) in [
        ("PATCH", Some(json!({"status":"purchased"}))),
        ("PATCH", Some(json!({"qty":2}))),
        ("DELETE", None),
    ] {
        let (s, _, e) = call(&pool, m, &url, &ck(&st), b).await;
        assert_eq!(
            (s, e["code"].as_str()),
            (StatusCode::NOT_FOUND, Some("NOT_FOUND")),
            "{m} {e}"
        );
    }

    // delivered 後捐助者不可取消、不可退回 purchased
    let h = ck(&tok);
    let patch = |b| call(&pool, "PATCH", &url, &h, Some(b));
    assert_eq!(patch(json!({"status":"delivered"})).await.0, StatusCode::OK);
    for b in [
        json!({"status":"purchased"}),
        json!({"status":"cancelled"}),
        json!({"qty":1}),
    ] {
        let (s, _, e) = patch(b.clone()).await;
        assert_eq!(
            (s, e["code"].as_str()),
            (StatusCode::CONFLICT, Some("INVALID_STATE_TRANSITION")),
            "{b} {e}"
        );
    }

    // cancelled 為終態
    let (tok2, c2) = user_claim(&pool, it, 1).await;
    let url2 = format!("/claims/{c2}");
    assert_eq!(
        call(&pool, "DELETE", &url2, &ck(&tok2), None).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(&pool, "DELETE", &url2, &ck(&tok2), None).await.0,
        StatusCode::NO_CONTENT
    );
    // 已取消時改量為 no-op，回目前（cancelled）資料：併發刪除品項時的既定行為，見 concurrency.rs force_delete_vs_claim_update
    let (s, _, e) = call(&pool, "PATCH", &url2, &ck(&tok2), Some(json!({"qty":2}))).await;
    assert_eq!(
        (s, e["claim"]["status"].as_str()),
        (StatusCode::OK, Some("cancelled")),
        "{e}"
    );
    for b in [json!({"status":"purchased"}), json!({"status":"delivered"})] {
        let (s, _, e) = call(&pool, "PATCH", &url2, &ck(&tok2), Some(b.clone())).await;
        assert_eq!(
            (s, e["code"].as_str()),
            (StatusCode::CONFLICT, Some("INVALID_STATE_TRANSITION")),
            "{b} {e}"
        );
    }
    assert_claims_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn claim_gates_crowdfund_and_hidden(pool: PgPool) {
    let TestUser { id: o, .. } = user(&pool, "o").await;
    let TestUser { token, .. } = user(&pool, "d").await;
    let post = |it: Uuid| {
        let h = [ck(&token), idem(Uuid::new_v4())].concat();
        let pool = pool.clone();
        async move {
            call(
                &pool,
                "POST",
                &format!("/items/{it}/claims"),
                &h,
                Some(json!({"qty":1})),
            )
            .await
        }
    };
    // 對眾籌品項做數量捐助 → FUNDING_MODE_MISMATCH
    let wl = wishlist(&pool, o, ListOpts::default()).await;
    let cf = crowdfund_item(&pool, wl.id, 1000).await;
    let (s, _, e) = post(cf).await;
    assert_eq!(
        (s, e["code"].as_str()),
        (StatusCode::CONFLICT, Some("FUNDING_MODE_MISMATCH")),
        "{e}"
    );
    // 被下架的清單 → 410 WISHLIST_REMOVED，且不寫入
    let hidden = wishlist(
        &pool,
        o,
        ListOpts {
            moderation_status: Some("hidden"),
            ..Default::default()
        },
    )
    .await;
    let it = item(&pool, hidden.id, "奶瓶", 2).await;
    let (s, _, e) = post(it).await;
    assert_eq!(
        (s, e["code"].as_str()),
        (StatusCode::GONE, Some("WISHLIST_REMOVED")),
        "{e}"
    );
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM claims WHERE item_id=$1")
        .bind(it)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 0);
}
