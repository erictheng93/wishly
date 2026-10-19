//! 儲值：建單規則（面額 / method / 登入 / 冪等 / 上限 / 新帳號每日上限 / 每小時限制 / 發票）、
//! 綠界表單簽章，以及 webhook（驗簽、金額核對、QueryTradeInfo 二次確認、冪等入帳、逾期後付款、失敗通知）、
//! PaymentInfoURL 取號通知、GET /wallet/topups 與 expire_due。QueryTradeInfo 由測試內的 axum mock 回應（見 `mock()`）。
mod common;
use axum::{
    body::Body,
    http::{HeaderMap, Request, StatusCode},
};
use axum::{extract::State, response::IntoResponse, routing::post, Form, Router};
use common::*;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};
use tokio::sync::Notify;
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

// ---- QueryTradeInfo mock（整個測試 binary 共用一台，依 MerchantTradeNo 區分各測試）----

/// 查詢期間停住，讓測試能在「確認呼叫進行中」檢查資料庫鎖。
#[derive(Default)]
struct Gate {
    entered: Notify,
    release: Notify,
}

#[derive(Clone)]
enum Reply {
    /// 200 + 簽章正確的 URL-encoded 查詢結果；mtn / tn 可覆寫成不一致的值，bad_sig 讓簽章錯誤
    Info {
        status: &'static str,
        amount: i64,
        mtn: Option<String>,
        tn: Option<String>,
        bad_sig: bool,
    },
    Http(u16),
    Hang,
    Gate(Arc<Gate>, i64),
}

fn paid(amount: i64) -> Reply {
    Reply::Info {
        status: "1",
        amount,
        mtn: None,
        tn: None,
        bad_sig: false,
    }
}

#[derive(Default)]
struct Mock {
    replies: Mutex<HashMap<String, Reply>>,
    calls: Mutex<HashMap<String, usize>>,
}

impl Mock {
    fn arm(&self, no: &str, r: Reply) {
        self.replies.lock().unwrap().insert(no.into(), r);
    }
    fn calls(&self, no: &str) -> usize {
        self.calls.lock().unwrap().get(no).copied().unwrap_or(0)
    }
}

fn trade_info_body(no: &str, status: &str, amount: i64, tn: &str, bad_sig: bool) -> String {
    let mut p: Vec<(String, String)> = [
        ("MerchantID", "3002607"),
        ("MerchantTradeNo", no),
        ("StoreID", ""),
        ("TradeNo", tn),
        ("TradeAmt", &amount.to_string()),
        ("PaymentDate", "2026/10/08 13:00:03"),
        ("PaymentType", "Credit_CreditCard"),
        ("HandlingCharge", "0"),
        ("PaymentTypeChargeFee", "0"),
        ("TradeDate", "2026/10/08 12:58:10"),
        ("TradeStatus", status),
        ("ItemName", "Wishly 點數"),
        ("CustomField", ""),
    ]
    .iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();
    let m = if bad_sig { "0".repeat(64) } else { mac(&p) };
    p.push(("CheckMacValue".into(), m));
    form_body(&p)
}

async fn query_handler(
    State(m): State<Arc<Mock>>,
    Form(p): Form<Vec<(String, String)>>,
) -> axum::response::Response {
    let get = |k: &str| {
        p.iter()
            .find(|(n, _)| n == k)
            .map(|(_, v)| v.clone())
            .unwrap_or_default()
    };
    // 我方送出的查詢本身也要簽章正確
    if get("CheckMacValue") != mac(&p)
        || get("MerchantID") != "3002607"
        || get("TimeStamp").parse::<i64>().is_err()
    {
        return (StatusCode::BAD_REQUEST, "bad query request").into_response();
    }
    let no = get("MerchantTradeNo");
    *m.calls.lock().unwrap().entry(no.clone()).or_default() += 1;
    let reply = m.replies.lock().unwrap().get(&no).cloned();
    tokio::time::sleep(Duration::from_millis(30)).await; // 讓併發通知確實重疊
    let tn = format!("2610{}", &no[1..]);
    match reply {
        None => (StatusCode::NOT_FOUND, "no such trade").into_response(),
        Some(Reply::Http(c)) => StatusCode::from_u16(c).unwrap().into_response(),
        Some(Reply::Hang) => {
            tokio::time::sleep(Duration::from_secs(30)).await;
            StatusCode::OK.into_response()
        }
        Some(Reply::Gate(g, amount)) => {
            g.entered.notify_one();
            g.release.notified().await;
            trade_info_body(&no, "1", amount, &tn, false).into_response()
        }
        Some(Reply::Info {
            status,
            amount,
            mtn,
            tn: tn2,
            bad_sig,
        }) => trade_info_body(
            mtn.as_deref().unwrap_or(&no),
            status,
            amount,
            tn2.as_deref().unwrap_or(&tn),
            bad_sig,
        )
        .into_response(),
    }
}

/// 啟動 mock 並把 config 的 ECPAY query_url 指過去（逾時縮短為 1 秒）。每個測試的第一個 ecpay 呼叫前都會經過這裡。
fn mock() -> &'static Arc<Mock> {
    static MOCK: OnceLock<Arc<Mock>> = OnceLock::new();
    MOCK.get_or_init(|| {
        let m = Arc::new(Mock::default());
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
        // sqlx::test 每個測試各有 runtime；此伺服器要比它們都活得久
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
    mock();
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
    mock();
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

/// 付款成功通知；未特別設定 mock 時，QueryTradeInfo 回「已付款、金額相同」。
async fn notify(pool: &PgPool, no: &str, amount: i64) -> (StatusCode, HeaderMap, String) {
    mock()
        .replies
        .lock()
        .unwrap()
        .entry(no.into())
        .or_insert_with(|| paid(amount));
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
    assert_eq!(v["limit_amount"], 50_000);
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
        assert!(f("PaymentInfoURL")
            .unwrap()
            .ends_with("/api/v1/webhooks/payments/ecpay/info"));
        assert!(f("ClientBackURL").unwrap().ends_with("/me/topup"));
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
    let (s, _, body) = notify_raw(&pool, &no, 500).await;
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
    assert_eq!(mock().calls(&no), 0, "通知金額不符時不必（也不該）去查綠界");

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
    assert_eq!(mock().calls(&no), 1, "已入帳的重送不再查綠界");
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

// ---- QueryTradeInfo 二次確認 ----

async fn audit_count(pool: &PgPool, action: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE action = $1")
        .bind(action)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn query_confirm_mismatch_does_not_credit_and_is_retryable(pool: PgPool) {
    let cases: Vec<(&str, Reply)> = vec![
        (
            "未付款 TradeStatus=0",
            Reply::Info {
                status: "0",
                amount: 1000,
                mtn: None,
                tn: None,
                bad_sig: false,
            },
        ),
        (
            "交易失敗 TradeStatus=10200095",
            Reply::Info {
                status: "10200095",
                amount: 1000,
                mtn: None,
                tn: None,
                bad_sig: false,
            },
        ),
        (
            "金額不符",
            Reply::Info {
                status: "1",
                amount: 999,
                mtn: None,
                tn: None,
                bad_sig: false,
            },
        ),
        (
            "MerchantTradeNo 不符",
            Reply::Info {
                status: "1",
                amount: 1000,
                mtn: Some("Tsomeoneelse".into()),
                tn: None,
                bad_sig: false,
            },
        ),
        (
            "TradeNo 不符",
            Reply::Info {
                status: "1",
                amount: 1000,
                mtn: None,
                tn: Some("9999999999".into()),
                bad_sig: false,
            },
        ),
        (
            "回應簽章錯誤",
            Reply::Info {
                status: "1",
                amount: 1000,
                mtn: None,
                tn: None,
                bad_sig: true,
            },
        ),
    ];
    for (i, (why, reply)) in cases.into_iter().enumerate() {
        let u = aged_user(&pool, "儲值者").await;
        let (_, _, v) = order(&pool, &u.token, 1000, "card").await;
        let no = trade_no(&pool, v["topup"]["id"].as_str().unwrap()).await;
        mock().arm(&no, reply);
        let (s, _, body) = notify(&pool, &no, 1000).await;
        assert_eq!(
            s,
            StatusCode::UNPROCESSABLE_ENTITY,
            "二次確認被略過：{why}：{body}"
        );
        assert!(body.contains("PAYMENT_QUERY_MISMATCH"), "{why}: {body}");
        assert_eq!(balance(&pool, u.id).await, 0, "二次確認被略過：{why}");
        assert_eq!(status_of(&pool, &no).await, "pending", "{why}");
        assert_eq!(
            audit_count(&pool, "topup.query_mismatch").await,
            i as i64 + 1,
            "{why}"
        );
        // 綠界重送且這次查詢結果正確 → 入帳
        mock().arm(&no, paid(1000));
        let (s, _, body) = notify(&pool, &no, 1000).await;
        assert_eq!((s, body.as_str()), (StatusCode::OK, "1|OK"), "{why}");
        assert_eq!(balance(&pool, u.id).await, 1000, "{why}");
    }
    assert_points_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn query_failure_returns_5xx_without_credit_then_retry_succeeds(pool: PgPool) {
    let u = aged_user(&pool, "儲值者").await;
    for (why, reply) in [
        ("HTTP 500", Reply::Http(500)),
        ("HTTP 404", Reply::Http(404)),
        ("逾時", Reply::Hang),
    ] {
        let (_, _, v) = order(&pool, &u.token, 500, "card").await;
        let no = trade_no(&pool, v["topup"]["id"].as_str().unwrap()).await;
        mock().arm(&no, reply);
        let (s, _, body) = notify_raw(&pool, &no, 500).await;
        assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE, "{why}: {body}");
        assert!(body.contains("PAYMENT_QUERY_FAILED"), "{why}: {body}");
        assert_eq!(status_of(&pool, &no).await, "pending", "{why}");
        mock().arm(&no, paid(500));
        assert_eq!(notify(&pool, &no, 500).await.2, "1|OK", "{why}");
        assert_eq!(status_of(&pool, &no).await, "paid", "{why}");
    }
    assert_eq!(balance(&pool, u.id).await, 1500);
    assert_eq!(
        audit_count(&pool, "topup.query_mismatch").await,
        0,
        "暫時性失敗不算不符"
    );
    assert_points_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn query_unknown_trade_returns_503(pool: PgPool) {
    // 綠界查無此單（HTTP 非 2xx）：與逾時相同，回 5xx 讓綠界重送，不入帳
    let u = aged_user(&pool, "儲值者").await;
    let (_, _, v) = order(&pool, &u.token, 500, "card").await;
    let no = trade_no(&pool, v["topup"]["id"].as_str().unwrap()).await;
    // 不 arm：mock 回 404（等同查無此單）
    let (s, _, _) = notify_raw(&pool, &no, 500).await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(balance(&pool, u.id).await, 0);
}

/// 不預設 mock 回應的通知（查詢會得到 404）。
async fn notify_raw(pool: &PgPool, no: &str, amount: i64) -> (StatusCode, HeaderMap, String) {
    post_form(pool, "ecpay", &notice(no, "1", amount)).await
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn query_runs_outside_the_row_lock_transaction(pool: PgPool) {
    let u = aged_user(&pool, "儲值者").await;
    let (_, _, v) = order(&pool, &u.token, 800, "card").await;
    let no = trade_no(&pool, v["topup"]["id"].as_str().unwrap()).await;
    let gate = Arc::new(Gate::default());
    mock().arm(&no, Reply::Gate(gate.clone(), 800));
    let (p, n) = (pool.clone(), no.clone());
    let pending = tokio::spawn(async move { notify(&p, &n, 800).await });
    tokio::time::timeout(Duration::from_secs(5), gate.entered.notified())
        .await
        .expect("確認呼叫應該已送出");
    // 確認進行中：同一列可被另一個連線更新（沒有被 FOR UPDATE 佔住），且沒有任何開著的交易
    tokio::time::timeout(
        Duration::from_secs(2),
        sqlx::query("UPDATE topups SET failure_reason = 'probe' WHERE merchant_trade_no = $1")
            .bind(&no)
            .execute(&pool),
    )
    .await
    .expect("確認呼叫期間 topups 列不應被鎖住")
    .unwrap();
    let open_tx: i64 = count(
        &pool,
        "SELECT count(*) FROM pg_stat_activity WHERE datname = current_database() AND state = 'idle in transaction'",
    )
    .await;
    assert_eq!(open_tx, 0, "確認呼叫期間不應持有開啟的資料庫交易");
    gate.release.notify_one();
    let (s, _, body) = pending.await.unwrap();
    assert_eq!((s, body.as_str()), (StatusCode::OK, "1|OK"));
    assert_eq!(balance(&pool, u.id).await, 800);
    assert_points_reconciled(&pool).await;
}

// ---- PaymentInfoURL 取號通知 ----

fn info_notice(no: &str, rtn: &str, amount: i64, extra: &[(&str, &str)]) -> Vec<(String, String)> {
    let mut p: Vec<(String, String)> = [
        ("MerchantID", "3002607"),
        ("MerchantTradeNo", no),
        ("RtnCode", rtn),
        ("RtnMsg", "Get Code Succeeded"),
        ("TradeAmt", &amount.to_string()),
        ("TradeDate", "2026/10/08 13:00:03"),
    ]
    .iter()
    .chain(extra.iter())
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();
    let m = mac(&p);
    p.push(("CheckMacValue".into(), m));
    p
}

async fn info_post(pool: &PgPool, p: &[(String, String)]) -> (StatusCode, String) {
    let (s, _, b) = post_form(pool, "ecpay/info", p).await;
    (s, b)
}

async fn topup_of(pool: &PgPool, token: &str, id: &str) -> Value {
    let (s, h, v) = call(
        pool,
        "GET",
        &format!("/wallet/topups/{id}"),
        &ck(token),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(h.get("cache-control").unwrap(), "private, no-store");
    v["topup"].clone()
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn payment_info_notice_stores_cvs_barcode_and_atm_details(pool: PgPool) {
    let u = aged_user(&pool, "儲值者").await;
    let cases = [
        (
            "cvs_code",
            "10100073",
            vec![
                ("PaymentType", "CVS_CVS"),
                ("PaymentNo", "LLL22101012345"),
                ("ExpireDate", "2026/10/22 23:59:59"),
            ],
            json!({ "type": "cvs_code", "payment_no": "LLL22101012345", "expire_at": "2026-10-22T15:59:59Z" }),
        ),
        (
            "cvs_barcode",
            "10100073",
            vec![
                ("PaymentType", "BARCODE_BARCODE"),
                ("Barcode1", "1110922"),
                ("Barcode2", "ABC1234567890123"),
                ("Barcode3", "110922000001000"),
                ("ExpireDate", "2026/10/22 12:30:00"),
            ],
            json!({ "type": "cvs_barcode", "barcode1": "1110922", "barcode2": "ABC1234567890123",
                    "barcode3": "110922000001000", "expire_at": "2026-10-22T04:30:00Z" }),
        ),
        (
            "atm",
            "2",
            vec![
                ("PaymentType", "ATM_TAISHIN"),
                ("BankCode", "812"),
                ("vAccount", "9103522175887271"),
                ("ExpireDate", "2026/10/22"),
            ],
            json!({ "type": "atm", "bank_code": "812", "account": "9103522175887271", "expire_at": "2026-10-22T15:59:59Z" }),
        ),
    ];
    for (method, rtn, extra, want) in cases {
        let (_, _, v) = order(&pool, &u.token, 1500, method).await;
        let id = v["topup"]["id"].as_str().unwrap().to_string();
        let no = trade_no(&pool, &id).await;
        assert!(topup_of(&pool, &u.token, &id).await["payment_info"].is_null());
        let n = info_notice(&no, rtn, 1500, &extra);
        assert_eq!(
            info_post(&pool, &n).await,
            (StatusCode::OK, "1|OK".into()),
            "{method}"
        );
        let t = topup_of(&pool, &u.token, &id).await;
        assert_eq!(t["payment_info"], want, "{method}");
        assert_eq!(t["status"], "pending");
        // 重送冪等
        assert_eq!(info_post(&pool, &n).await.1, "1|OK");
        assert_eq!(topup_of(&pool, &u.token, &id).await["payment_info"], want);
    }
    assert_eq!(balance(&pool, u.id).await, 0, "取號通知不入帳");
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM topups WHERE payment_info IS NOT NULL"
        )
        .await,
        3
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn payment_info_notice_rules(pool: PgPool) {
    let u = aged_user(&pool, "儲值者").await;
    let (_, _, v) = order(&pool, &u.token, 1500, "cvs_code").await;
    let id = v["topup"]["id"].as_str().unwrap().to_string();
    let no = trade_no(&pool, &id).await;
    let extra = [("PaymentNo", "AAA111"), ("ExpireDate", "not a date")];

    // 簽章錯 → 401；竄改後不寫入
    let mut forged = info_notice(&no, "10100073", 1500, &extra);
    forged.iter_mut().find(|(k, _)| k == "TradeAmt").unwrap().1 = "1".into();
    assert_eq!(info_post(&pool, &forged).await.0, StatusCode::UNAUTHORIZED);
    // 金額不符 → 422 + audit，不寫入
    let (s, _) = info_post(&pool, &info_notice(&no, "10100073", 1400, &extra)).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(audit_count(&pool, "topup.info_amount_mismatch").await, 1);
    assert!(topup_of(&pool, &u.token, &id).await["payment_info"].is_null());
    // 訂單不存在 → 404；tappay → 501
    let (s, _) = info_post(&pool, &info_notice("Tnope", "10100073", 1500, &extra)).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert_eq!(
        post_form(&pool, "tappay/info", &[]).await.0,
        StatusCode::NOT_IMPLEMENTED
    );
    // 欄位不齊（缺 PaymentNo）或 RtnCode=1（那是付款通知）→ 回 OK 但不寫入
    assert_eq!(
        info_post(&pool, &info_notice(&no, "10100073", 1500, &[]))
            .await
            .1,
        "1|OK"
    );
    assert_eq!(
        info_post(&pool, &info_notice(&no, "1", 1500, &extra))
            .await
            .1,
        "1|OK"
    );
    assert!(topup_of(&pool, &u.token, &id).await["payment_info"].is_null());

    // ExpireDate 無法解析 → 退回 topups.expires_at
    assert_eq!(
        info_post(&pool, &info_notice(&no, "10100073", 1500, &extra))
            .await
            .1,
        "1|OK"
    );
    let t = topup_of(&pool, &u.token, &id).await;
    let exp: String = sqlx::query_scalar(
        "SELECT to_char(expires_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') FROM topups WHERE id = $1::uuid",
    )
    .bind(&id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        t["payment_info"],
        json!({ "type": "cvs_code", "payment_no": "AAA111", "expire_at": exp })
    );

    // 已 paid 後的取號通知不覆寫
    assert_eq!(notify(&pool, &no, 1500).await.2, "1|OK");
    let other = [
        ("PaymentNo", "ZZZ999"),
        ("ExpireDate", "2026/10/22 23:59:59"),
    ];
    assert_eq!(
        info_post(&pool, &info_notice(&no, "10100073", 1500, &other))
            .await
            .1,
        "1|OK"
    );
    let t = topup_of(&pool, &u.token, &id).await;
    assert_eq!(t["status"], "paid");
    assert_eq!(t["payment_info"]["payment_no"], "AAA111");
    assert_points_reconciled(&pool).await;
}

// ---- GET /wallet/topups、GET /wallet/topups/{id} ----

#[sqlx::test(migrations = "../../db/migrations")]
async fn topup_queries_are_owner_only_newest_first_with_limit(pool: PgPool) {
    let a = aged_user(&pool, "甲").await;
    let b = aged_user(&pool, "乙").await;
    assert_eq!(
        call(&pool, "GET", "/wallet/topups", &[], None).await.0,
        StatusCode::UNAUTHORIZED
    );
    let fake = Uuid::new_v4();
    assert_eq!(
        call(&pool, "GET", &format!("/wallet/topups/{fake}"), &[], None)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    let (s, h, v) = call(&pool, "GET", "/wallet/topups", &ck(&a.token), None).await;
    assert_eq!(
        (s, v["topups"].as_array().unwrap().len()),
        (StatusCode::OK, 0)
    );
    assert_eq!(h.get("cache-control").unwrap(), "private, no-store");

    let mut ids = vec![];
    for (amt, m) in [(100, "card"), (200, "atm"), (300, "cvs_code")] {
        let (_, _, v) = order(&pool, &a.token, amt, m).await;
        ids.push(v["topup"]["id"].as_str().unwrap().to_string());
    }
    let (_, _, bv) = order(&pool, &b.token, 999, "card").await;
    let b_id = bv["topup"]["id"].as_str().unwrap().to_string();

    let (_, _, v) = call(&pool, "GET", "/wallet/topups", &ck(&a.token), None).await;
    let list = v["topups"].as_array().unwrap();
    assert_eq!(
        list.iter()
            .map(|t| t["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec![ids[2].as_str(), ids[1].as_str(), ids[0].as_str()],
        "最新在前，且不含他人"
    );
    let t = &list[0];
    let mut keys: Vec<&str> = t.as_object().unwrap().keys().map(String::as_str).collect();
    keys.sort();
    assert_eq!(
        keys,
        [
            "amount",
            "created_at",
            "expires_at",
            "id",
            "method",
            "paid_at",
            "payment_info",
            "status"
        ]
    );
    assert_eq!(
        (
            t["amount"].as_i64(),
            t["method"].as_str(),
            t["status"].as_str()
        ),
        (Some(300), Some("cvs_code"), Some("pending"))
    );
    assert!(t["paid_at"].is_null() && t["payment_info"].is_null());

    // 單筆：本人 200；他人 / 不存在 404
    assert_eq!(topup_of(&pool, &a.token, &ids[0]).await["amount"], 100);
    for id in [b_id.as_str(), fake.to_string().as_str()] {
        let (s, _, v) = call(
            &pool,
            "GET",
            &format!("/wallet/topups/{id}"),
            &ck(&a.token),
            None,
        )
        .await;
        assert_eq!(
            (s, v["code"].as_str()),
            (StatusCode::NOT_FOUND, Some("NOT_FOUND")),
            "{id}"
        );
    }
    assert!(!call(
        &pool,
        "GET",
        "/wallet/topups/not-a-uuid",
        &ck(&a.token),
        None
    )
    .await
    .0
    .is_success());

    // 付款後 status / paid_at
    let no = trade_no(&pool, &ids[0]).await;
    assert_eq!(notify(&pool, &no, 100).await.2, "1|OK");
    let t = topup_of(&pool, &a.token, &ids[0]).await;
    assert_eq!(t["status"], "paid");
    assert!(t["paid_at"].is_string());

    // limit：預設 20、上限 50、下限 1
    sqlx::query(
        "INSERT INTO topups (wallet_id, merchant_trade_no, provider, method, amount, expires_at)
         SELECT w.id, 'Tbulk' || g, 'ecpay', 'card', 100, now() + interval '1 day'
           FROM point_wallets w, generate_series(1, 57) g WHERE w.user_id = $1",
    )
    .bind(a.id)
    .execute(&pool)
    .await
    .unwrap();
    for (q, want) in [
        ("", 20),
        ("?limit=5", 5),
        ("?limit=50", 50),
        ("?limit=1000", 50),
        ("?limit=0", 1),
        ("?limit=-3", 1),
    ] {
        let (s, _, v) = call(
            &pool,
            "GET",
            &format!("/wallet/topups{q}"),
            &ck(&a.token),
            None,
        )
        .await;
        assert_eq!(
            (s, v["topups"].as_array().unwrap().len()),
            (StatusCode::OK, want),
            "{q}"
        );
    }
    let (s, _, _) = call(
        &pool,
        "GET",
        "/wallet/topups?limit=abc",
        &ck(&a.token),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

/// 跨網域前端（Pages）要能讀到 429 的 Retry-After，CORS 必須公開該標頭
#[sqlx::test(migrations = "../../db/migrations")]
async fn cors_exposes_retry_after(pool: PgPool) {
    let origin = wishly_api::config::get().app_url;
    let (_, h, _) = call(
        &pool,
        "GET",
        "/public/wishlists",
        &[("origin", origin)],
        None,
    )
    .await;
    let exposed = h["access-control-expose-headers"]
        .to_str()
        .unwrap()
        .to_lowercase();
    assert!(
        exposed.contains("retry-after") && exposed.contains("idempotency-replayed"),
        "{exposed}"
    );
}
