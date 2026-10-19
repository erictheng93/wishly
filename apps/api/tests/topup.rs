//! 儲值：建單規則（面額 / 上限 / 登入）與 webhook（驗簽、金額核對、冪等入帳、與 ledger 對帳）
mod common;
use axum::http::StatusCode;
use common::*;
use hmac::{Hmac, Mac};
use serde_json::{json, Value};
use sha2::Sha256;
use sqlx::PgPool;

const SECRET: &str = "dev-topup-webhook-secret-0123456789";

fn sign(body: &Value) -> Hdr {
    let mut m = Hmac::<Sha256>::new_from_slice(SECRET.as_bytes()).unwrap();
    m.update(body.to_string().as_bytes());
    vec![("x-topup-signature", hex::encode(m.finalize().into_bytes()))]
}

async fn order(pool: &PgPool, token: &str, amount: i64) -> Resp {
    call(
        pool,
        "POST",
        "/topups",
        &ck(token),
        Some(json!({ "amount": amount })),
    )
    .await
}

async fn notify(pool: &PgPool, trade_no: &str, amount: i64, status: &str) -> Resp {
    let b = json!({ "merchant_trade_no": trade_no, "provider_trade_no": format!("P-{trade_no}"),
                    "amount": amount, "status": status });
    call(pool, "POST", "/webhooks/topup/ecpay", &sign(&b), Some(b)).await
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn create_validates_amount_login_and_cap(pool: PgPool) {
    let u = user(&pool, "儲值者").await;
    let (s, _, _) = call(
        &pool,
        "POST",
        "/topups",
        &[],
        Some(json!({ "amount": 500 })),
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    assert_eq!(
        order(&pool, &u.token, 123).await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let (s, _, v) = order(&pool, &u.token, 500).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    assert_eq!(v["status"], "pending");
    assert_eq!(balance(&pool, u.id).await, 0, "建單不入帳");
    // 餘額 + 待付款 + 新單 > 50,000 → 409
    grant(&pool, u.id, 47_000).await;
    let (s, _, v) = order(&pool, &u.token, 5000).await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["code"], "WALLET_CAP_EXCEEDED");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn webhook_credits_once_and_checks_signature_and_amount(pool: PgPool) {
    let u = user(&pool, "儲值者").await;
    let (_, _, v) = order(&pool, &u.token, 1000).await;
    let no = v["merchant_trade_no"].as_str().unwrap().to_string();

    // 簽章錯誤 → 401，不入帳
    let b = json!({ "merchant_trade_no": no, "provider_trade_no": "X", "amount": 1000, "status": "paid" });
    let bad = vec![("x-topup-signature", "00".repeat(32))];
    assert_eq!(
        call(&pool, "POST", "/webhooks/topup/ecpay", &bad, Some(b))
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    // 金額不符 → 422，不入帳
    let (s, _, v) = notify(&pool, &no, 500, "paid").await;
    assert_eq!(
        (s, v["code"].as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, Some("AMOUNT_MISMATCH"))
    );
    assert_eq!(balance(&pool, u.id).await, 0);
    // 付款成功 → 入帳；重送冪等
    assert_eq!(notify(&pool, &no, 1000, "paid").await.0, StatusCode::OK);
    let (s, _, v) = notify(&pool, &no, 1000, "paid").await;
    assert_eq!((s, &v["replayed"]), (StatusCode::OK, &json!(true)));
    assert_eq!(balance(&pool, u.id).await, 1000);
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM point_ledger WHERE entry_type = 'topup'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 1);
    // 不存在的訂單 → 404
    assert_eq!(
        notify(&pool, "WLnope", 1000, "paid").await.0,
        StatusCode::NOT_FOUND
    );
    assert_points_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn concurrent_duplicate_notices_credit_once(pool: PgPool) {
    let u = user(&pool, "儲值者").await;
    let (_, _, v) = order(&pool, &u.token, 2000).await;
    let no = v["merchant_trade_no"].as_str().unwrap().to_string();
    let rs = futures_util::future::join_all((0..8).map(|_| notify(&pool, &no, 2000, "paid"))).await;
    assert!(rs.iter().all(|r| r.0 == StatusCode::OK));
    assert_eq!(balance(&pool, u.id).await, 2000);
    assert_points_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn late_payment_after_expiry_still_credits(pool: PgPool) {
    let u = user(&pool, "儲值者").await;
    let (_, _, v) = order(&pool, &u.token, 500).await;
    let no = v["merchant_trade_no"].as_str().unwrap().to_string();
    sqlx::query("UPDATE topups SET expires_at = now() - interval '1 day'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(notify(&pool, &no, 500, "paid").await.0, StatusCode::OK);
    assert_eq!(balance(&pool, u.id).await, 500);
}
