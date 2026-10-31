//! 金流通知發送點：儲值入帳 / 逾期、現金退款、錢包凍結、爭議、眾籌達標 / 退點 / 採購單。重點是冪等：重送不多寄。
mod common;
use axum::{
    body::Body,
    extract::State,
    http::{Request, StatusCode},
    response::IntoResponse,
    routing::post,
    Form, Router,
};
use common::*;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
};
use tower::ServiceExt;
use uuid::Uuid;
use wishly_api::wallet_ops;

const KEY: &str = "pwFHCqoQZGmho4w6";
const IV: &str = "EkRm7iFT261dpevs";

async fn n(pool: &PgPool, user: Uuid, kind: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM notifications WHERE user_id = $1 AND kind = $2")
        .bind(user)
        .bind(kind)
        .fetch_one(pool)
        .await
        .unwrap()
}

// ---- 綠界：簽章與 QueryTradeInfo mock（精簡版，見 topup.rs）----
fn mac(params: &[(String, String)]) -> String {
    let mut p: Vec<_> = params
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

fn signed(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    let mut p: Vec<(String, String)> = pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let m = mac(&p);
    p.push(("CheckMacValue".into(), m));
    p
}

type Amounts = Arc<Mutex<HashMap<String, i64>>>;

async fn query_handler(
    State(m): State<Amounts>,
    Form(p): Form<Vec<(String, String)>>,
) -> axum::response::Response {
    let no = p
        .iter()
        .find(|(k, _)| k == "MerchantTradeNo")
        .map(|(_, v)| v.clone())
        .unwrap_or_default();
    let Some(amount) = m.lock().unwrap().get(&no).copied() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let tn = format!("2610{}", &no[1..]);
    form_body(&signed(&[
        ("MerchantID", "3002607"),
        ("MerchantTradeNo", &no),
        ("StoreID", ""),
        ("TradeNo", &tn),
        ("TradeAmt", &amount.to_string()),
        ("PaymentDate", "2026/10/08 13:00:03"),
        ("PaymentType", "Credit_CreditCard"),
        ("HandlingCharge", "0"),
        ("PaymentTypeChargeFee", "0"),
        ("TradeDate", "2026/10/08 12:58:10"),
        ("TradeStatus", "1"),
        ("ItemName", "Wishly 點數"),
        ("CustomField", ""),
    ]))
    .into_response()
}

fn mock() -> &'static Amounts {
    static M: OnceLock<Amounts> = OnceLock::new();
    M.get_or_init(|| {
        let m: Amounts = Default::default();
        let router = Router::new()
            .route("/Cashier/QueryTradeInfo/V5", post(query_handler))
            .with_state(m.clone());
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let mut c = wishly_api::config::Config::dev();
        let e = c.ecpay.as_mut().unwrap();
        e.query_url = format!(
            "http://{}/Cashier/QueryTradeInfo/V5",
            listener.local_addr().unwrap()
        );
        e.query_timeout_secs = 1;
        wishly_api::config::init(c);
        std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async move {
                    axum::serve(tokio::net::TcpListener::from_std(listener).unwrap(), router)
                        .await
                        .unwrap();
                });
        });
        m
    })
}

/// 建一筆 pending 儲值單，回傳 (topup_id, merchant_trade_no)
async fn pending_topup(pool: &PgPool, u: &TestUser, amount: i64) -> (Uuid, String) {
    mock();
    sqlx::query("UPDATE users SET created_at = now() - interval '60 days' WHERE id = $1")
        .bind(u.id)
        .execute(pool)
        .await
        .unwrap();
    let mut h = ck(&u.token);
    h.extend(idem(Uuid::new_v4()));
    let (s, _, b) = call(
        pool,
        "POST",
        "/wallet/topups",
        &h,
        Some(json!({ "amount": amount, "method": "card" })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let id: Uuid = b["topup"]["id"].as_str().unwrap().parse().unwrap();
    let no = sqlx::query_scalar("SELECT merchant_trade_no FROM topups WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap();
    (id, no)
}

async fn pay_webhook(pool: &PgPool, no: &str, amount: i64) -> StatusCode {
    mock().lock().unwrap().insert(no.into(), amount);
    let a = amount.to_string();
    let tn = format!("2610{}", &no[1..]);
    let p = signed(&[
        ("MerchantID", "3002607"),
        ("MerchantTradeNo", no),
        ("RtnCode", "1"),
        ("RtnMsg", "交易成功"),
        ("TradeNo", &tn),
        ("TradeAmt", &a),
        ("PaymentDate", "2026/10/08 13:00:03"),
        ("PaymentType", "Credit_CreditCard"),
    ]);
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/webhooks/payments/ecpay")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from(form_body(&p)))
        .unwrap();
    test_app(pool).oneshot(req).await.unwrap().status()
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn topup_credited_once_and_worker_sends(pool: PgPool) {
    let u = user(&pool, "儲值者").await;
    let (_, no) = pending_topup(&pool, &u, 500).await;
    assert_eq!(pay_webhook(&pool, &no, 500).await, StatusCode::OK);
    assert_eq!(pay_webhook(&pool, &no, 500).await, StatusCode::OK); // 重送
    assert_eq!(n(&pool, u.id, "topup.credited").await, 1);
    let p: Value = sqlx::query_scalar(
        "SELECT payload FROM notifications WHERE user_id = $1 AND kind = 'topup.credited'",
    )
    .bind(u.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(p["amount"], 500);
    wishly_api::notify::tick_at(&pool, noon()).await.unwrap();
    let st: String = sqlx::query_scalar(
        "SELECT status::text FROM notifications WHERE user_id = $1 AND kind = 'topup.credited'",
    )
    .bind(u.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_ne!(st, "failed");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn topup_expired_only_when_payment_info_present(pool: PgPool) {
    let u = user(&pool, "儲值者").await;
    let (a, _) = pending_topup(&pool, &u, 500).await;
    let (b, _) = pending_topup(&pool, &u, 600).await;
    sqlx::query("UPDATE topups SET expires_at = now() - interval '1 minute' WHERE id = ANY($1)")
        .bind(vec![a, b])
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE topups SET payment_info = '{}'::jsonb WHERE id = $1")
        .bind(a)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(wishly_api::topup::expire_due(&pool).await.unwrap(), 2);
    assert_eq!(wishly_api::topup::expire_due(&pool).await.unwrap(), 0);
    assert_eq!(n(&pool, u.id, "topup.expired").await, 1);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn cash_refund_result_notified_once(pool: PgPool) {
    let s = staff(&pool, "營運").await;
    let a = user(&pool, "甲").await;
    topup_credit(&pool, a.id, 100).await;
    for (to, extra) in [
        ("completed", json!({})),
        ("rejected", json!({ "reject_reason": "資料不符" })),
    ] {
        let (st, _, b) = call(
            &pool,
            "POST",
            "/wallet/cash-refunds",
            &[ck(&a.token), idem(Uuid::new_v4())].concat(),
            Some(json!({ "amount": 10 })),
        )
        .await;
        assert_eq!(st, StatusCode::CREATED, "{b}");
        let url = format!(
            "/admin/cash-refunds/{}",
            b["request"]["id"].as_str().unwrap()
        );
        let h = ck(&s.token);
        let patch = |body: Value| call(&pool, "PATCH", &url, &h, Some(body));
        let before = n(&pool, a.id, "cash_refund.updated").await;
        let (st, ..) = patch(json!({ "status": "processing" })).await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(n(&pool, a.id, "cash_refund.updated").await, before); // processing 不通知
        let mut body = extra.clone();
        body["status"] = json!(to);
        assert_eq!(patch(body.clone()).await.0, StatusCode::OK);
        assert_eq!(patch(body).await.0, StatusCode::CONFLICT); // 重送
        assert_eq!(n(&pool, a.id, "cash_refund.updated").await, before + 1);
    }
    let notes: Vec<Value> = sqlx::query_scalar("SELECT payload FROM notifications WHERE user_id = $1 AND kind = 'cash_refund.updated' ORDER BY id")
        .bind(a.id).fetch_all(&pool).await.unwrap();
    assert_eq!(
        (
            notes[0]["status"].as_str(),
            notes[1]["status"].as_str(),
            notes[1]["note"].as_str()
        ),
        (Some("completed"), Some("rejected"), Some("資料不符"))
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn wallet_freeze_notified_only_on_change(pool: PgPool) {
    let s = staff(&pool, "營運").await;
    let a = user(&pool, "甲").await;
    grant(&pool, a.id, 10).await;
    let wid: Uuid = sqlx::query_scalar("SELECT id FROM point_wallets WHERE user_id = $1")
        .bind(a.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    let url = format!("/admin/wallets/{wid}");
    for (to, kind, expect) in [
        ("frozen", "wallet.frozen", 1),
        ("frozen", "wallet.frozen", 1),
        ("active", "wallet.unfrozen", 1),
        ("active", "wallet.unfrozen", 1),
    ] {
        let (st, _, b) = call(
            &pool,
            "PATCH",
            &url,
            &ck(&s.token),
            Some(json!({ "status": to, "reason": "測試" })),
        )
        .await;
        assert_eq!(st, StatusCode::OK, "{b}");
        assert_eq!(n(&pool, a.id, kind).await, expect);
    }
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn dispute_flow_notifications_are_idempotent(pool: PgPool) {
    let a = user(&pool, "甲").await;
    let t = topup_credit(&pool, a.id, 100).await;
    let run = |f: u8| {
        let pool = pool.clone();
        async move {
            let mut tx = pool.begin().await.unwrap();
            match f {
                0 => {
                    wallet_ops::dispute_open(&mut tx, t).await.unwrap();
                }
                1 => {
                    wallet_ops::dispute_won(&mut tx, t).await.unwrap();
                }
                _ => {
                    wallet_ops::charge_back(&mut tx, t).await.unwrap();
                }
            }
            tx.commit().await.unwrap();
        }
    };
    run(0).await;
    run(0).await;
    assert_eq!(n(&pool, a.id, "wallet.frozen").await, 1);
    run(1).await;
    run(1).await;
    assert_eq!(n(&pool, a.id, "wallet.unfrozen").await, 1);
    run(0).await;
    run(2).await;
    run(2).await;
    assert_eq!(n(&pool, a.id, "wallet.chargeback").await, 1);
    let p: Value = sqlx::query_scalar(
        "SELECT payload FROM notifications WHERE user_id = $1 AND kind = 'wallet.chargeback'",
    )
    .bind(a.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        (p["amount"].as_i64(), p["debt"].as_i64()),
        (Some(100), Some(0))
    );
}

// ---- 眾籌 ----
async fn shipped_list(pool: &PgPool, owner: Uuid) -> Uuid {
    let w = wishlist(pool, owner, ListOpts::default()).await.id;
    let sealed = |s: &str| wishly_api::sealed::seal(s.as_bytes());
    sqlx::query("INSERT INTO shipping_addresses (wishlist_id, recipient_name_enc, phone_enc, address_enc) VALUES ($1, $2, $3, $4)")
        .bind(w).bind(sealed("王小米")).bind(sealed("0912345678")).bind(sealed("台北市")).execute(pool).await.unwrap();
    w
}

async fn order_patch(pool: &PgPool, tok: &str, oid: Uuid, body: Value) -> StatusCode {
    call(
        pool,
        "PATCH",
        &format!("/admin/orders/{oid}"),
        &ck(tok),
        Some(body),
    )
    .await
    .0
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn funded_and_order_lifecycle_notify_owner_and_donors(pool: PgPool) {
    let app = test_app(&pool);
    let s = staff(&pool, "營運").await;
    let owner = user(&pool, "小米").await;
    let a = user(&pool, "甲").await;
    let b = user(&pool, "乙").await;
    grant(&pool, a.id, 1000).await;
    grant(&pool, b.id, 1000).await;
    let wl = shipped_list(&pool, owner.id).await;
    let item = crowdfund_item(&pool, wl, 500).await;
    assert_eq!(
        pledge(&app, item, &a.token, 300).await.0,
        StatusCode::CREATED
    );
    assert_eq!(
        pledge(&app, item, &b.token, 200).await.0,
        StatusCode::CREATED
    );
    assert_eq!(n(&pool, owner.id, "owner.funded").await, 1);
    let oid: Uuid = sqlx::query_scalar("SELECT id FROM purchase_orders WHERE item_id = $1")
        .bind(item)
        .fetch_one(&pool)
        .await
        .unwrap();

    // 實際花費 400 < 目標 500：差額退回，捐贈者收到 partial_refund；重送被擋，不多寄
    let placed = json!({ "status": "placed", "merchant_order_id": "M1", "amount": 400 });
    assert_eq!(
        order_patch(&pool, &s.token, oid, placed.clone()).await,
        StatusCode::OK
    );
    assert_eq!(
        order_patch(&pool, &s.token, oid, placed).await,
        StatusCode::CONFLICT
    );
    assert_eq!(
        (
            n(&pool, a.id, "order.partial_refund").await,
            n(&pool, b.id, "order.partial_refund").await
        ),
        (1, 1)
    );

    assert_eq!(
        order_patch(&pool, &s.token, oid, json!({ "status": "shipped" })).await,
        StatusCode::OK
    );
    assert_eq!(
        order_patch(&pool, &s.token, oid, json!({ "status": "shipped" })).await,
        StatusCode::CONFLICT
    );
    assert_eq!(n(&pool, owner.id, "owner.shipped").await, 1);
    assert_eq!(
        order_patch(&pool, &s.token, oid, json!({ "status": "delivered" })).await,
        StatusCode::OK
    );
    assert_eq!(n(&pool, owner.id, "owner.delivered").await, 1);
    assert_eq!(n(&pool, owner.id, "owner.order_failed").await, 0);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn full_amount_has_no_partial_refund_and_failed_notifies_owner(pool: PgPool) {
    let app = test_app(&pool);
    let s = staff(&pool, "營運").await;
    let owner = user(&pool, "小米").await;
    let a = user(&pool, "甲").await;
    grant(&pool, a.id, 1000).await;
    let wl = shipped_list(&pool, owner.id).await;
    let item = crowdfund_item(&pool, wl, 500).await;
    pledge(&app, item, &a.token, 500).await;
    let oid: Uuid = sqlx::query_scalar("SELECT id FROM purchase_orders WHERE item_id = $1")
        .bind(item)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        order_patch(
            &pool,
            &s.token,
            oid,
            json!({ "status": "placed", "merchant_order_id": "M1", "amount": 500 })
        )
        .await,
        StatusCode::OK
    );
    assert_eq!(n(&pool, a.id, "order.partial_refund").await, 0);
    let failed = json!({ "status": "failed", "failure_reason": "缺貨" });
    assert_eq!(
        order_patch(&pool, &s.token, oid, failed.clone()).await,
        StatusCode::OK
    );
    assert_eq!(
        order_patch(&pool, &s.token, oid, failed).await,
        StatusCode::CONFLICT
    );
    assert_eq!(n(&pool, owner.id, "owner.order_failed").await, 1);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn auto_release_notifies_pledgers_once(pool: PgPool) {
    let app = test_app(&pool);
    let owner = user(&pool, "小米").await;
    let a = user(&pool, "甲").await;
    grant(&pool, a.id, 1000).await;
    let wl = shipped_list(&pool, owner.id).await;
    let item = crowdfund_item(&pool, wl, 500).await;
    pledge(&app, item, &a.token, 100).await;
    sqlx::query(
        "UPDATE wishlist_items SET funding_deadline = now() - interval '8 days' WHERE id = $1",
    )
    .bind(item)
    .execute(&pool)
    .await
    .unwrap();
    wishly_api::contributions::tick_funding(&pool)
        .await
        .unwrap(); // open -> expired
    assert_eq!(n(&pool, a.id, "funding.released").await, 0);
    sqlx::query("UPDATE wishlist_items SET expired_at = now() - interval '8 days' WHERE id = $1")
        .bind(item)
        .execute(&pool)
        .await
        .unwrap();
    wishly_api::contributions::tick_funding(&pool)
        .await
        .unwrap();
    wishly_api::contributions::tick_funding(&pool)
        .await
        .unwrap(); // 再跑一次：已無 pledged
    assert_eq!(n(&pool, a.id, "funding.released").await, 1);
}
