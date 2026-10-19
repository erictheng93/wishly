//! 儲值：建單規則（面額 / method / 登入 / 冪等 / 上限 / 新帳號每日上限 / 每小時限制 / 發票）、
//! 綠界表單簽章，以及 webhook（驗簽、金額核對、冪等入帳、逾期後付款、失敗通知）與 expire_due。
mod common;
use axum::{
    body::Body,
    http::{HeaderMap, Request, StatusCode},
};
use common::*;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

// 綠界公開測試參數（與 config dev 預設相同）
const KEY: &str = "pwFHCqoQZGmho4w6";
const IV: &str = "EkRm7iFT261dpevs";

/// 獨立於 topup.rs 的簽章實作：排序後逐字元編碼（.NET UrlEncode 風格）。
fn mac(params: &[(String, String)]) -> String {
    let mut p: Vec<(String, String)> = params
        .iter()
        .filter(|(k, _)| k != "CheckMacValue")
        .cloned()
        .collect();
    p.sort_by_key(|a| a.0.to_lowercase());
    let mut raw = format!("HashKey={KEY}");
    for (k, v) in &p {
        raw += &format!("&{k}={v}");
    }
    raw += &format!("&HashIV={IV}");
    let mut enc = String::new();
    for b in raw.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!*()".contains(&b) {
            enc.push(b as char);
        } else if b == b' ' {
            enc.push('+');
        } else {
            enc += &format!("%{b:02X}");
        }
    }
    hex::encode(Sha256::digest(enc.to_lowercase())).to_uppercase()
}

fn form_body(params: &[(String, String)]) -> String {
    let e = |s: &str| {
        s.bytes()
            .map(|b| {
                if b.is_ascii_alphanumeric() {
                    (b as char).to_string()
                } else {
                    format!("%{b:02X}")
                }
            })
            .collect::<String>()
    };
    params
        .iter()
        .map(|(k, v)| format!("{}={}", e(k), e(v)))
        .collect::<Vec<_>>()
        .join("&")
}

async fn aged_user(pool: &PgPool, name: &str) -> TestUser {
    let u = user(pool, name).await;
    sqlx::query("UPDATE users SET created_at = now() - interval '60 days' WHERE id = $1")
        .bind(u.id)
        .execute(pool)
        .await
        .unwrap();
    u
}

fn hdrs(token: &str, key: Uuid) -> Hdr {
    let mut h = ck(token);
    h.extend(idem(key));
    h
}

async fn order_with(pool: &PgPool, token: &str, body: Value) -> Resp {
    call(
        pool,
        "POST",
        "/wallet/topups",
        &hdrs(token, Uuid::new_v4()),
        Some(body),
    )
    .await
}

async fn order(pool: &PgPool, token: &str, amount: i64, method: &str) -> Resp {
    order_with(pool, token, json!({ "amount": amount, "method": method })).await
}

async fn trade_no(pool: &PgPool, topup_id: &str) -> String {
    sqlx::query_scalar("SELECT merchant_trade_no FROM topups WHERE id = $1::uuid")
        .bind(topup_id)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn post_form(
    pool: &PgPool,
    provider: &str,
    params: &[(String, String)],
) -> (StatusCode, HeaderMap, String) {
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/webhooks/payments/{provider}"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from(form_body(params)))
        .unwrap();
    let res = test_app(pool).oneshot(req).await.unwrap();
    let (st, h) = (res.status(), res.headers().clone());
    let b = res.into_body().collect().await.unwrap().to_bytes();
    (st, h, String::from_utf8_lossy(&b).into_owned())
}

fn notice(no: &str, rtn: &str, amount: i64) -> Vec<(String, String)> {
    let mut p: Vec<(String, String)> = [
        ("MerchantID", "3002607"),
        ("MerchantTradeNo", no),
        ("RtnCode", rtn),
        ("RtnMsg", "交易成功"),
        ("TradeNo", &format!("2610{}", &no[1..])),
        ("TradeAmt", &amount.to_string()),
        ("PaymentDate", "2026/10/08 13:00:03"),
        ("PaymentType", "Credit_CreditCard"),
    ]
    .iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();
    let m = mac(&p);
    p.push(("CheckMacValue".into(), m));
    p
}

async fn notify(pool: &PgPool, no: &str, amount: i64) -> (StatusCode, HeaderMap, String) {
    post_form(pool, "ecpay", &notice(no, "1", amount)).await
}

async fn status_of(pool: &PgPool, no: &str) -> String {
    sqlx::query_scalar("SELECT status::text FROM topups WHERE merchant_trade_no = $1")
        .bind(no)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn count(pool: &PgPool, sql: &str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(pool).await.unwrap()
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn create_validates_login_key_amount_and_method(pool: PgPool) {
    let u = aged_user(&pool, "儲值者").await;
    let body = Some(json!({ "amount": 500, "method": "card" }));
    assert_eq!(
        call(
            &pool,
            "POST",
            "/wallet/topups",
            &idem(Uuid::new_v4()),
            body.clone()
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    let (s, _, v) = call(&pool, "POST", "/wallet/topups", &ck(&u.token), body).await;
    assert_eq!(
        (s, v["code"].as_str()),
        (StatusCode::BAD_REQUEST, Some("IDEMPOTENCY_KEY_REQUIRED"))
    );
    for bad in [
        json!({ "amount": 99, "method": "card" }),
        json!({ "amount": 50_001, "method": "card" }),
        json!({ "method": "card" }),
        json!({ "amount": 500 }),
        json!({ "amount": 500, "method": "bitcoin" }),
    ] {
        assert_eq!(
            order_with(&pool, &u.token, bad.clone()).await.0,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{bad}"
        );
    }
    let (s, _, v) = order(&pool, &u.token, 500, "line_pay").await;
    assert_eq!(
        (s, v["errors"][0]["code"].as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, Some("UNSUPPORTED"))
    );
    // 邊界值都可建立
    for a in [100, 50_000] {
        let u = aged_user(&pool, "邊界").await;
        let (s, _, v) = order(&pool, &u.token, a, "card").await;
        assert_eq!(s, StatusCode::CREATED, "{v}");
    }
    let (s, _, v) = order(&pool, &u.token, 500, "card").await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    assert_eq!(v["topup"]["status"], "pending");
    assert_eq!(v["topup"]["provider"], "ecpay");
    assert_eq!(v["topup"]["amount"], 500);
    assert_eq!(v["payment"]["type"], "form_post");
    assert_eq!(balance(&pool, u.id).await, 0, "建單不入帳");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn create_replays_same_key_and_rejects_changed_body(pool: PgPool) {
    let u = aged_user(&pool, "儲值者").await;
    let key = Uuid::new_v4();
    let b = json!({ "amount": 500, "method": "atm" });
    let (s1, _, v1) = call(
        &pool,
        "POST",
        "/wallet/topups",
        &hdrs(&u.token, key),
        Some(b.clone()),
    )
    .await;
    let (s2, h2, v2) = call(
        &pool,
        "POST",
        "/wallet/topups",
        &hdrs(&u.token, key),
        Some(b),
    )
    .await;
    assert_eq!((s1, s2), (StatusCode::CREATED, StatusCode::CREATED));
    assert_eq!(h2.get("idempotency-replayed").unwrap(), "true");
    assert_eq!(v1, v2);
    assert_eq!(count(&pool, "SELECT count(*) FROM topups").await, 1);
    assert_eq!(count(&pool, "SELECT count(*) FROM invoices").await, 1);
    let (s, _, v) = call(
        &pool,
        "POST",
        "/wallet/topups",
        &hdrs(&u.token, key),
        Some(json!({ "amount": 600, "method": "atm" })),
    )
    .await;
    assert_eq!(
        (s, v["code"].as_str()),
        (StatusCode::CONFLICT, Some("IDEMPOTENCY_CONFLICT"))
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn wallet_cap_counts_balance_and_pending(pool: PgPool) {
    let u = aged_user(&pool, "儲值者").await;
    grant(&pool, u.id, 47_000).await;
    let (s, _, v) = order(&pool, &u.token, 5000, "card").await;
    assert_eq!(
        (s, v["code"].as_str()),
        (StatusCode::CONFLICT, Some("WALLET_CAP_EXCEEDED")),
        "{v}"
    );
    assert_eq!(
        order(&pool, &u.token, 3000, "card").await.0,
        StatusCode::CREATED
    );
    // 待付款也計入
    let (s, _, v) = order(&pool, &u.token, 100, "card").await;
    assert_eq!(
        (s, v["code"].as_str()),
        (StatusCode::CONFLICT, Some("WALLET_CAP_EXCEEDED"))
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn new_account_daily_limit(pool: PgPool) {
    let n = user(&pool, "新帳號").await;
    assert_eq!(
        order(&pool, &n.token, 2000, "card").await.0,
        StatusCode::CREATED
    );
    let (s, _, v) = order(&pool, &n.token, 1500, "card").await;
    assert_eq!(
        (s, v["code"].as_str()),
        (StatusCode::CONFLICT, Some("TOPUP_LIMIT_EXCEEDED")),
        "{v}"
    );
    assert_eq!(
        (v["limit_amount"].as_i64(), v["remaining_amount"].as_i64()),
        (Some(3000), Some(1000))
    );
    assert_eq!(
        order(&pool, &n.token, 1000, "card").await.0,
        StatusCode::CREATED
    );
    // 超過 30 天的帳號不受限
    let old = aged_user(&pool, "老帳號").await;
    assert_eq!(
        order(&pool, &old.token, 5000, "card").await.0,
        StatusCode::CREATED
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn hourly_limit_is_10_with_retry_after(pool: PgPool) {
    let u = aged_user(&pool, "儲值者").await;
    for _ in 0..10 {
        assert_eq!(
            order(&pool, &u.token, 100, "card").await.0,
            StatusCode::CREATED
        );
    }
    let (s, h, v) = order(&pool, &u.token, 100, "card").await;
    assert_eq!(
        (s, v["code"].as_str()),
        (StatusCode::TOO_MANY_REQUESTS, Some("RATE_LIMITED"))
    );
    assert!(
        h.get("retry-after")
            .unwrap()
            .to_str()
            .unwrap()
            .parse::<u64>()
            .unwrap()
            > 0
    );
    assert_eq!(
        count(&pool, "SELECT count(*) FROM topups").await,
        10,
        "被拒的請求不留痕"
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn invoice_options_are_validated_and_stored(pool: PgPool) {
    let u = aged_user(&pool, "儲值者").await;
    // 預設會員載具
    let (s, _, v) = order(&pool, &u.token, 500, "card").await;
    assert_eq!(s, StatusCode::CREATED);
    let (ct, st, src, linked): (String, String, String, bool) = sqlx::query_as(
        "SELECT i.carrier_type::text, i.status::text, i.source_type::text, t.invoice_id = i.id
           FROM topups t JOIN invoices i ON i.source_id = t.id WHERE t.id = $1::uuid",
    )
    .bind(v["topup"]["id"].as_str())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        (ct.as_str(), st.as_str(), src.as_str(), linked),
        ("member", "pending", "topup", true)
    );
    for inv in [
        json!({ "carrier_type": "mobile_barcode", "carrier_id": "/ABC1234" }),
        json!({ "carrier_type": "citizen_cert", "carrier_id": "AB12345678901234" }),
        json!({ "carrier_type": "donate", "donate_code": "168001" }),
        json!({ "carrier_type": "b2b", "buyer_tax_id": "12345678", "buyer_name": "某某公司" }),
    ] {
        let (s, _, v) = order_with(
            &pool,
            &u.token,
            json!({ "amount": 500, "method": "card", "invoice": inv }),
        )
        .await;
        assert_eq!(s, StatusCode::CREATED, "{inv}: {v}");
    }
    let n = count(&pool, "SELECT count(*) FROM invoices").await;
    assert_eq!(n, 5);
    let carrier: String =
        sqlx::query_scalar("SELECT carrier_id FROM invoices WHERE carrier_type = 'mobile_barcode'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(carrier, "/ABC1234");
    for inv in [
        json!({ "carrier_type": "mobile_barcode" }),
        json!({ "carrier_type": "mobile_barcode", "carrier_id": "ABC1234" }),
        json!({ "carrier_type": "citizen_cert", "carrier_id": "x" }),
        json!({ "carrier_type": "donate" }),
        json!({ "carrier_type": "b2b", "buyer_tax_id": "1234567" }),
        json!({ "carrier_type": "paper" }),
        json!("string"),
    ] {
        let (s, _, v) = order_with(
            &pool,
            &u.token,
            json!({ "amount": 500, "method": "card", "invoice": inv }),
        )
        .await;
        assert_eq!(
            (s, v["code"].as_str()),
            (StatusCode::UNPROCESSABLE_ENTITY, Some("VALIDATION_FAILED")),
            "{inv}"
        );
    }
    assert_eq!(
        count(&pool, "SELECT count(*) FROM invoices").await,
        n,
        "驗證失敗不寫發票"
    );
    assert_eq!(count(&pool, "SELECT count(*) FROM topups").await, n);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn payment_form_is_signed_per_method(pool: PgPool) {
    let u = aged_user(&pool, "儲值者").await;
    for (method, choose, expire) in [
        ("card", "Credit", None),
        ("cvs_code", "CVS", Some(("StoreExpireDate", "4320"))),
        ("cvs_barcode", "BARCODE", Some(("StoreExpireDate", "3"))),
        ("atm", "ATM", Some(("ExpireDate", "3"))),
    ] {
        let (s, _, v) = order(&pool, &u.token, 1234, method).await;
        assert_eq!(s, StatusCode::CREATED, "{v}");
        let p = &v["payment"];
        assert!(p["url"].as_str().unwrap().contains("AioCheckOut"));
        let fields: Vec<(String, String)> = p["fields"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.as_str().unwrap().to_string()))
            .collect();
        let f = |k: &str| fields.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
        assert_eq!(f("CheckMacValue"), Some(mac(&fields).as_str()), "{method}");
        assert_eq!(f("MerchantID"), Some("3002607"));
        let no = trade_no(&pool, v["topup"]["id"].as_str().unwrap()).await;
        assert_eq!(f("MerchantTradeNo"), Some(no.as_str()));
        assert_eq!(
            (
                f("TotalAmount"),
                f("ChoosePayment"),
                f("PaymentType"),
                f("EncryptType")
            ),
            (Some("1234"), Some(choose), Some("aio"), Some("1"))
        );
        assert!(f("ReturnURL")
            .unwrap()
            .ends_with("/api/v1/webhooks/payments/ecpay"));
        assert!(f("ClientBackURL").is_some());
        assert_eq!(f("MerchantTradeDate").unwrap().len(), 19);
        if let Some((k, val)) = expire {
            assert_eq!(f(k), Some(val), "{method}");
        }
    }
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn webhook_verifies_signature_amount_and_credits_once(pool: PgPool) {
    let u = aged_user(&pool, "儲值者").await;
    let (_, _, v) = order(&pool, &u.token, 1000, "card").await;
    let id = v["topup"]["id"].as_str().unwrap();
    let no = trade_no(&pool, id).await;

    // 簽章錯誤（竄改金額 / 無簽章）→ 401，不入帳
    let mut forged = notice(&no, "1", 1000);
    forged.iter_mut().find(|(k, _)| k == "TradeAmt").unwrap().1 = "999999".into();
    assert_eq!(
        post_form(&pool, "ecpay", &forged).await.0,
        StatusCode::UNAUTHORIZED
    );
    let mut unsigned = notice(&no, "1", 1000);
    unsigned.retain(|(k, _)| k != "CheckMacValue");
    assert_eq!(
        post_form(&pool, "ecpay", &unsigned).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(balance(&pool, u.id).await, 0);

    // 金額不符（簽章正確）→ 不入帳、寫 audit_logs，仍為 pending
    let (s, _, body) = notify(&pool, &no, 500).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(balance(&pool, u.id).await, 0);
    assert_eq!(status_of(&pool, &no).await, "pending");
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM audit_logs WHERE action = 'topup.amount_mismatch'"
        )
        .await,
        1
    );

    // 成功 → 純文字 1|OK 並入帳；重送冪等
    let (s, h, body) = notify(&pool, &no, 1000).await;
    assert_eq!((s, body.as_str()), (StatusCode::OK, "1|OK"));
    assert!(h
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap()
        .starts_with("text/plain"));
    assert_eq!(balance(&pool, u.id).await, 1000);
    let (s, _, body) = notify(&pool, &no, 1000).await;
    assert_eq!((s, body.as_str()), (StatusCode::OK, "1|OK"));
    assert_eq!(balance(&pool, u.id).await, 1000);
    let (st, txn, paid): (String, String, bool) = sqlx::query_as(
        "SELECT status::text, provider_txn_id, paid_at IS NOT NULL FROM topups WHERE id = $1::uuid",
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        (st.as_str(), txn.starts_with("2610"), paid),
        ("paid", true, true)
    );
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM point_ledger WHERE entry_type = 'topup'"
        )
        .await,
        1
    );
    // 已 paid 後收到失敗通知不改狀態
    assert_eq!(
        post_form(&pool, "ecpay", &notice(&no, "0", 1000)).await.2,
        "1|OK"
    );
    assert_eq!(status_of(&pool, &no).await, "paid");
    // 不存在的訂單 → 404；tappay 尚未實作 → 501
    assert_eq!(notify(&pool, "Tnope", 1000).await.0, StatusCode::NOT_FOUND);
    assert_eq!(
        post_form(&pool, "tappay", &[]).await.0,
        StatusCode::NOT_IMPLEMENTED
    );
    assert_points_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn concurrent_duplicate_notices_credit_once(pool: PgPool) {
    let u = aged_user(&pool, "儲值者").await;
    let (_, _, v) = order(&pool, &u.token, 2000, "card").await;
    let no = trade_no(&pool, v["topup"]["id"].as_str().unwrap()).await;
    let rs = futures_util::future::join_all((0..8).map(|_| notify(&pool, &no, 2000))).await;
    let rs: Vec<(StatusCode, ())> = rs.into_iter().map(|r| (r.0, ())).collect();
    assert!(rs.iter().all(|r| r.0 == StatusCode::OK));
    assert_eq!(balance(&pool, u.id).await, 2000);
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM point_ledger WHERE entry_type = 'topup'"
        )
        .await,
        1
    );
    assert_points_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn late_payment_after_expiry_still_credits(pool: PgPool) {
    let u = aged_user(&pool, "儲值者").await;
    let (_, _, v) = order(&pool, &u.token, 500, "atm").await;
    let no = trade_no(&pool, v["topup"]["id"].as_str().unwrap()).await;
    sqlx::query("UPDATE topups SET expires_at = now() - interval '1 day'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(wishly_api::topup::expire_due(&pool).await.unwrap(), 1);
    assert_eq!(status_of(&pool, &no).await, "expired");
    assert_eq!(notify(&pool, &no, 500).await.2, "1|OK");
    assert_eq!(status_of(&pool, &no).await, "paid");
    assert_eq!(balance(&pool, u.id).await, 500);
    assert_points_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn failed_notice_marks_failed_without_credit(pool: PgPool) {
    let u = aged_user(&pool, "儲值者").await;
    let (_, _, v) = order(&pool, &u.token, 500, "card").await;
    let no = trade_no(&pool, v["topup"]["id"].as_str().unwrap()).await;
    assert_eq!(
        post_form(&pool, "ecpay", &notice(&no, "10100058", 500))
            .await
            .2,
        "1|OK"
    );
    assert_eq!(status_of(&pool, &no).await, "failed");
    assert_eq!(balance(&pool, u.id).await, 0);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn expire_due_only_expires_overdue_pending(pool: PgPool) {
    let u = aged_user(&pool, "儲值者").await;
    let (_, _, a) = order(&pool, &u.token, 500, "card").await;
    let (_, _, b) = order(&pool, &u.token, 600, "card").await;
    let an = trade_no(&pool, a["topup"]["id"].as_str().unwrap()).await;
    let bn = trade_no(&pool, b["topup"]["id"].as_str().unwrap()).await;
    sqlx::query(
        "UPDATE topups SET expires_at = now() - interval '1 minute' WHERE merchant_trade_no = $1",
    )
    .bind(&an)
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(wishly_api::topup::expire_due(&pool).await.unwrap(), 1);
    assert_eq!(status_of(&pool, &an).await, "expired");
    assert_eq!(status_of(&pool, &bn).await, "pending");
    assert_eq!(wishly_api::topup::expire_due(&pool).await.unwrap(), 0);
    // 過期單不再占用錢包上限，未過期的仍計入
    grant(&pool, u.id, 49_000).await;
    let (s, _, v) = order(&pool, &u.token, 500, "card").await;
    assert_eq!(
        (s, v["code"].as_str()),
        (StatusCode::CONFLICT, Some("WALLET_CAP_EXCEEDED"))
    );
    assert_eq!(
        order(&pool, &u.token, 400, "card").await.0,
        StatusCode::CREATED
    );
}
