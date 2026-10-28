//! F17 × F18：未結案的現金退款、拒付欠款、凍結錢包都要擋刪帳號（流程驗證 W3）。
mod common;
use axum::http::StatusCode;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use common::*;

async fn delete_me(pool: &PgPool, token: &str) -> (StatusCode, serde_json::Value) {
    let (s, _, b) = call(
        pool,
        "DELETE",
        "/me",
        &ck(token),
        Some(json!({ "confirm": "DELETE" })),
    )
    .await;
    (s, b)
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn pending_cash_refund_blocks_account_deletion(pool: PgPool) {
    let TestUser { id, token } = user(&pool, "甲").await;
    topup_credit(&pool, id, 100).await;
    // 全額申請退款：點數先扣，餘額 0；若之後被拒，點數會退回這個錢包
    let (s, _, b) = call(
        &pool,
        "POST",
        "/wallet/cash-refunds",
        &[ck(&token), idem(Uuid::new_v4())].concat(),
        Some(json!({ "amount": 100 })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    assert_eq!(balance(&pool, id).await, 0);
    let (s, b) = delete_me(&pool, &token).await;
    assert_eq!(
        (s, b["code"].as_str()),
        (StatusCode::CONFLICT, Some("ACCOUNT_HAS_POINTS")),
        "{b}"
    );
    // 結案（completed）後即可刪除
    sqlx::query("UPDATE cash_refund_requests SET status = 'completed'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(delete_me(&pool, &token).await.0, StatusCode::OK);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn open_debt_or_frozen_wallet_blocks_account_deletion(pool: PgPool) {
    let TestUser { id, token } = user(&pool, "乙").await;
    let topup = topup_credit(&pool, id, 100).await;
    // 模擬拒付敗訴：點數已花光（餘額 0），欠款 open，錢包凍結
    sqlx::query("UPDATE point_wallets SET status = 'frozen' WHERE user_id = $1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    let (s, b) = delete_me(&pool, &token).await;
    assert_eq!(
        (s, b["code"].as_str()),
        (StatusCode::CONFLICT, Some("ACCOUNT_HAS_POINTS")),
        "frozen + balance {b}"
    );
    let mut tx = pool.begin().await.unwrap();
    let wallet: Uuid = sqlx::query_scalar("SELECT id FROM point_wallets WHERE user_id = $1")
        .bind(id)
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    wishly_api::points::post(
        &mut tx,
        wallet,
        -100,
        wishly_api::points::Entry {
            kind: "adjustment",
            ref_type: "topup",
            ref_id: Some(topup),
            note: None,
            actor: None,
        },
    )
    .await
    .unwrap();
    sqlx::query("INSERT INTO wallet_debts (wallet_id, topup_id, amount) VALUES ($1, $2, 40)")
        .bind(wallet)
        .bind(topup)
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(balance(&pool, id).await, 0);

    let (s, b) = delete_me(&pool, &token).await;
    assert_eq!(
        (s, b["code"].as_str()),
        (StatusCode::CONFLICT, Some("ACCOUNT_HAS_POINTS")),
        "debt + frozen {b}"
    );
    // 只解凍、欠款仍 open → 仍擋
    sqlx::query("UPDATE point_wallets SET status = 'active' WHERE id = $1")
        .bind(wallet)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(delete_me(&pool, &token).await.0, StatusCode::CONFLICT);
    // 欠款核銷後可刪
    sqlx::query("UPDATE wallet_debts SET status = 'written_off', resolved_at = now()")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(delete_me(&pool, &token).await.0, StatusCode::OK);
    assert_points_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn pending_topup_blocks_account_deletion_until_not_pending(pool: PgPool) {
    let TestUser { id, token } = user(&pool, "丙").await;
    sqlx::query("INSERT INTO point_wallets (user_id) VALUES ($1)")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO topups (wallet_id, merchant_trade_no, provider, method, amount, expires_at)
        SELECT id, 'Tpend1', 'ecpay', 'atm', 100, now() + interval '1 day' FROM point_wallets WHERE user_id = $1")
        .bind(id).execute(&pool).await.unwrap();
    let (s, b) = delete_me(&pool, &token).await;
    assert_eq!(
        (s, b["code"].as_str()),
        (StatusCode::CONFLICT, Some("ACCOUNT_HAS_POINTS")),
        "{b}"
    );
    // 過了我方期限但還是 pending（尚未被 expire_due 翻轉）仍須擋住：可能還能付款
    sqlx::query("UPDATE topups SET expires_at = now() - interval '1 minute'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(delete_me(&pool, &token).await.0, StatusCode::CONFLICT);
    sqlx::query("UPDATE topups SET status = 'expired'")
        .execute(&pool)
        .await
        .unwrap();
    let (s, b) = delete_me(&pool, &token).await;
    assert_eq!(s, StatusCode::OK, "{b}");
}
