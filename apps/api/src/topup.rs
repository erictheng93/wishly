//! 儲值（docs/04 `POST /wallet/topups`、`POST /webhooks/payments/{provider}`，D53 / D54）。
//! 1 點 = NT$1。目前只實作 provider=ecpay（綠界 AioCheckOut，form_post）；tappay 與 line_pay / apple_pay 尚未實作。
//! 建單：Idempotency-Key 必填，占位與寫入同一交易；入帳只在 webhook，且只走 `points::post`（kind=topup）。
//! 憑證（ECPAY_*）未設齊時兩個端點都回 503 TOPUP_DISABLED。
//! webhook 入帳前以 QueryTradeInfo 二次確認（D56）：外部 HTTP 在資料庫交易之外，入帳才開交易 FOR UPDATE 重讀訂單；
//! 取號通知（PaymentInfoURL）寫入 `topups.payment_info`。`GET /wallet/topups[/{id}]` 供前端顯示繳費資訊。
use crate::{
    config::{self, Ecpay},
    error::AppError,
    idempotency,
    points::{self, Entry},
    session::CurrentUser,
    AppState,
};
use axum::{
    body::Bytes,
    extract::{rejection::FormRejection, Form, Path, Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use chrono::{
    DateTime, Duration, FixedOffset, NaiveDate, NaiveDateTime, SecondsFormat, TimeZone, Utc,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use subtle::ConstantTimeEq;
use uuid::Uuid;

/// 單次儲值金額範圍（NT$）。
pub const MIN_AMOUNT: i64 = 100;
pub const MAX_AMOUNT: i64 = 50_000;
/// 單一錢包餘額（含待付款）上限：電子支付機構管理條例的儲值上限 NT$50,000。
pub const WALLET_CAP: i64 = 50_000;
/// 單一使用者每小時最多建立的儲值單數。
const HOURLY_LIMIT: i64 = 10;
/// 新帳號（註冊 30 天內）每日（Asia/Taipei）儲值上限（4.14 / D47-2）。
const NEW_ACCOUNT_DAILY: i64 = 3000;
/// 信用卡付款頁有效分鐘；超商 / ATM 繳費期限天數。
const CARD_TTL_MIN: i64 = 30;
const OFFLINE_TTL_DAYS: i64 = 3;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/wallet/topups", post(create).get(list))
        .route("/wallet/topups/{id}", get(detail))
        .route("/webhooks/payments/{provider}", post(webhook))
        .route("/webhooks/payments/{provider}/info", post(webhook_info))
        .route(
            "/webhooks/payments/{provider}/dispute",
            post(webhook_dispute),
        )
}

fn ecpay() -> Result<Ecpay, AppError> {
    config::get()
        .ecpay
        .ok_or_else(|| AppError::problem(503, "TOPUP_DISABLED", "儲值功能尚未開放"))
}

/// 每分鐘把逾期未付的待付款單標為 expired。
pub fn spawn_expire(pool: PgPool) {
    tokio::spawn(async move {
        let mut t = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            t.tick().await;
            if let Err(e) = expire_due(&pool).await {
                tracing::error!(error=?e, "topup expire_due");
            }
        }
    });
}

/// pending 且已過期 → expired，回傳筆數。與 webhook 競爭時以列鎖序列化：webhook 先拿到鎖則此處看到已非 pending 而略過。
/// 已 expired 但事後收到付款成功通知，webhook 仍會入帳（錢已收）。
pub async fn expire_due(pool: &PgPool) -> Result<u64, AppError> {
    Ok(sqlx::query(
        "UPDATE topups SET status = 'expired', updated_at = now() WHERE status = 'pending' AND expires_at < now()",
    )
    .execute(pool)
    .await?
    .rows_affected())
}

// ---- 綠界 CheckMacValue ----

/// .NET 風格 URL encode：英數與 `-_.!*()` 不編碼、空白轉 `+`、其餘 %xx；整串最後轉小寫由呼叫端處理。
fn url_encode(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'a'..=b'z'
            | b'A'..=b'Z'
            | b'0'..=b'9'
            | b'-'
            | b'_'
            | b'.'
            | b'!'
            | b'*'
            | b'('
            | b')' => o.push(b as char),
            b' ' => o.push('+'),
            _ => o.push_str(&format!("%{b:02x}")),
        }
    }
    o
}

/// 排除 CheckMacValue、依參數名排序（不分大小寫），前後加 HashKey / HashIV，編碼轉小寫後 SHA-256 轉大寫。
fn check_mac(params: &[(String, String)], cfg: &Ecpay) -> String {
    let mut p: Vec<&(String, String)> = params
        .iter()
        .filter(|(k, _)| k != "CheckMacValue")
        .collect();
    p.sort_by_key(|(k, _)| k.to_lowercase());
    let body = p
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&");
    let raw = format!("HashKey={}&{body}&HashIV={}", cfg.hash_key, cfg.hash_iv);
    hex::encode_upper(Sha256::digest(url_encode(&raw).to_lowercase().as_bytes()))
}

fn payment_form(
    cfg: &Ecpay,
    trade_no: &str,
    amount: i64,
    method: &str,
    at: DateTime<Utc>,
) -> Value {
    let tw = FixedOffset::east_opt(8 * 3600).expect("UTC+8");
    let mut f: Vec<(String, String)> = vec![
        ("MerchantID".into(), cfg.merchant_id.clone()),
        ("MerchantTradeNo".into(), trade_no.into()),
        (
            "MerchantTradeDate".into(),
            at.with_timezone(&tw)
                .format("%Y/%m/%d %H:%M:%S")
                .to_string(),
        ),
        ("PaymentType".into(), "aio".into()),
        ("TotalAmount".into(), amount.to_string()),
        ("TradeDesc".into(), "Wishly 點數儲值".into()),
        ("ItemName".into(), format!("Wishly 點數 {amount} 點")),
        ("ReturnURL".into(), cfg.return_url.clone()),
        ("EncryptType".into(), "1".into()),
        ("PaymentInfoURL".into(), cfg.payment_info_url.clone()),
        (
            "ClientBackURL".into(),
            format!("{}/me/topup", config::get().app_url),
        ),
    ];
    let (choose, expire) = match method {
        "card" => ("Credit", None),
        "cvs_code" => ("CVS", Some(("StoreExpireDate", OFFLINE_TTL_DAYS * 24 * 60))), // 分鐘
        "cvs_barcode" => ("BARCODE", Some(("StoreExpireDate", OFFLINE_TTL_DAYS))),    // 天
        _ => ("ATM", Some(("ExpireDate", OFFLINE_TTL_DAYS))),                         // 天
    };
    f.push(("ChoosePayment".into(), choose.into()));
    if let Some((k, v)) = expire {
        f.push((k.into(), v.to_string()));
    }
    let mac = check_mac(&f, cfg);
    f.push(("CheckMacValue".into(), mac));
    json!({ "type": "form_post", "url": cfg.endpoint, "fields": f.into_iter().map(|(k, v)| (k, Value::String(v))).collect::<serde_json::Map<_, _>>() })
}

// ---- POST /wallet/topups ----

struct Invoice {
    carrier_type: String,
    carrier_id: Option<String>,
    donate_code: Option<String>,
    buyer_tax_id: Option<String>,
    buyer_name: Option<String>,
}

fn s(v: &Value, k: &str) -> Option<String> {
    v.get(k)
        .and_then(Value::as_str)
        .map(|x| x.trim().to_string())
        .filter(|x| !x.is_empty())
}

/// 預設會員載具；其餘載具欄位依 invoices_carrier_fields CHECK 驗證。
fn parse_invoice(v: Option<&Value>) -> Result<Invoice, AppError> {
    let empty = json!({});
    let v = v.filter(|v| !v.is_null()).unwrap_or(&empty);
    if !v.is_object() {
        return Err(AppError::invalid("/invoice", "TYPE", "invoice 必須是物件"));
    }
    let carrier_type = s(v, "carrier_type").unwrap_or_else(|| "member".into());
    let mut inv = Invoice {
        carrier_type,
        carrier_id: s(v, "carrier_id"),
        donate_code: s(v, "donate_code"),
        buyer_tax_id: s(v, "buyer_tax_id"),
        buyer_name: s(v, "buyer_name"),
    };
    let digits = |x: &str, lo: usize, hi: usize| {
        (lo..=hi).contains(&x.len()) && x.bytes().all(|b| b.is_ascii_digit())
    };
    let bad = |p: &str, d: &str| Err(AppError::invalid(p, "INVALID", d));
    match inv.carrier_type.as_str() {
        "member" => {
            (
                inv.carrier_id,
                inv.donate_code,
                inv.buyer_tax_id,
                inv.buyer_name,
            ) = (None, None, None, None);
        }
        "mobile_barcode" => match inv.carrier_id.as_deref() {
            // 手機條碼：/ + 7 碼（大寫英數與 . + -）
            Some(c)
                if c.len() == 8
                    && c.starts_with('/')
                    && c[1..].bytes().all(|b| {
                        b.is_ascii_uppercase() || b.is_ascii_digit() || b"+-.".contains(&b)
                    }) => {}
            _ => return bad("/invoice/carrier_id", "手機條碼格式為 / 加 7 碼大寫英數"),
        },
        "citizen_cert" => match inv.carrier_id.as_deref() {
            Some(c)
                if c.len() == 16
                    && c[..2].bytes().all(|b| b.is_ascii_uppercase())
                    && c[2..].bytes().all(|b| b.is_ascii_digit()) => {}
            _ => {
                return bad(
                    "/invoice/carrier_id",
                    "自然人憑證條碼為 2 碼大寫英文加 14 碼數字",
                )
            }
        },
        "donate" => match inv.donate_code.as_deref() {
            Some(c) if digits(c, 3, 7) => {}
            _ => return bad("/invoice/donate_code", "捐贈碼為 3–7 碼數字"),
        },
        "b2b" => match inv.buyer_tax_id.as_deref() {
            Some(c) if digits(c, 8, 8) => {}
            _ => return bad("/invoice/buyer_tax_id", "統一編號為 8 碼數字"),
        },
        _ => return bad("/invoice/carrier_type", "carrier_type 不支援"),
    }
    Ok(inv)
}

fn replayed(status: i32, body: Value) -> Response {
    let mut res = (
        StatusCode::from_u16(status as u16).unwrap_or(StatusCode::OK),
        Json(body),
    )
        .into_response();
    res.headers_mut()
        .insert("idempotency-replayed", HeaderValue::from_static("true"));
    res
}

async fn create(
    State(st): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, AppError> {
    let cfg = ecpay()?;
    let key = idempotency::key(&headers)?;
    let b: Value = serde_json::from_slice(&body)
        .map_err(|_| AppError::problem(400, "BAD_REQUEST", "請求內容必須是 JSON"))?;
    let amount = b
        .get("amount")
        .and_then(Value::as_i64)
        .filter(|a| (MIN_AMOUNT..=MAX_AMOUNT).contains(a))
        .ok_or_else(|| {
            AppError::invalid(
                "/amount",
                "RANGE",
                &format!("amount 必須是 {MIN_AMOUNT}–{MAX_AMOUNT} 的整數"),
            )
        })?;
    let method = match b.get("method").and_then(Value::as_str) {
        Some(m @ ("card" | "cvs_code" | "cvs_barcode" | "atm")) => m,
        Some("line_pay" | "apple_pay") => {
            return Err(AppError::invalid(
                "/method",
                "UNSUPPORTED",
                "此付款方式尚未支援",
            ))
        }
        _ => {
            return Err(AppError::invalid(
                "/method",
                "ENUM",
                "method 必須是 card | line_pay | apple_pay | cvs_code | cvs_barcode | atm",
            ))
        }
    };
    let inv = parse_invoice(b.get("invoice"))?;

    let scope = format!("user:{}:POST /wallet/topups", user.id);
    let hash = idempotency::request_hash("POST /wallet/topups", &body);
    let mut tx = st.pool.begin().await?;
    if let idempotency::Begin::Replay(status, body) =
        idempotency::begin(&mut tx, &scope, key, &hash).await?
    {
        return Ok(replayed(status, body));
    }
    let wallet = points::ensure_wallet(&mut tx, user.id).await?;
    points::lock_wallets(&mut tx, &[wallet]).await?; // 同使用者的建立請求序列化，上限檢查才不會被併發繞過

    // 每小時建單數
    let (n, oldest): (i64, Option<DateTime<Utc>>) = sqlx::query_as(
        "SELECT count(*), min(created_at) FROM topups WHERE wallet_id = $1 AND created_at > now() - interval '1 hour'",
    )
    .bind(wallet)
    .fetch_one(&mut *tx)
    .await?;
    if n >= HOURLY_LIMIT {
        let wait = oldest.map_or(60, |o| {
            (o + Duration::hours(1) - Utc::now()).num_seconds().max(1)
        });
        return Err(AppError::Problem {
            status: 429,
            code: "RATE_LIMITED",
            detail: format!("每小時最多建立 {HOURLY_LIMIT} 筆儲值單"),
            errors: None,
            retry_after: Some(wait as u64),
        });
    }
    // 錢包上限：餘額 + 未過期待付款 + 本次
    let held: i64 = sqlx::query_scalar(
        "SELECT (SELECT balance FROM point_wallets WHERE id = $1)
              + COALESCE((SELECT sum(amount) FROM topups WHERE wallet_id = $1 AND status = 'pending' AND expires_at > now()), 0)::bigint",
    )
    .bind(wallet)
    .fetch_one(&mut *tx)
    .await?;
    if held + amount > WALLET_CAP {
        return Err(AppError::Extra {
            status: 409,
            code: "WALLET_CAP_EXCEEDED",
            detail: format!("儲值後餘額將超過上限 {WALLET_CAP} 點"),
            extra: json!({ "limit_amount": WALLET_CAP }),
        });
    }
    // 4.14 新帳號每日上限（gift_card_orders 尚未存在，僅計儲值單）；0 rows = 非新帳號
    let remaining: Option<i64> = sqlx::query_scalar(
        "SELECT ($2 - COALESCE((SELECT sum(t.amount) FROM topups t
                                 WHERE t.wallet_id = $3 AND t.status IN ('pending', 'paid')
                                   AND t.created_at >= date_trunc('day', now() AT TIME ZONE 'Asia/Taipei') AT TIME ZONE 'Asia/Taipei'), 0))::bigint
           FROM users u WHERE u.id = $1 AND u.created_at > now() - interval '30 days'",
    )
    .bind(user.id)
    .bind(NEW_ACCOUNT_DAILY)
    .bind(wallet)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(r) = remaining.filter(|r| *r < amount) {
        return Err(AppError::Extra {
            status: 409,
            code: "TOPUP_LIMIT_EXCEEDED",
            detail: format!("新帳號每日儲值上限 NT${NEW_ACCOUNT_DAILY}"),
            extra: json!({ "limit_amount": NEW_ACCOUNT_DAILY, "remaining_amount": r.max(0) }),
        });
    }

    let trade_no = format!("T{}", &Uuid::new_v4().simple().to_string()[..19]); // 綠界訂單編號上限 20 字元
    let ttl = if method == "card" {
        Duration::minutes(CARD_TTL_MIN)
    } else {
        Duration::days(OFFLINE_TTL_DAYS)
    };
    let (id, created_at): (Uuid, DateTime<Utc>) = sqlx::query_as(
        "INSERT INTO topups (wallet_id, merchant_trade_no, provider, method, amount, expires_at)
         VALUES ($1, $2, 'ecpay', $3::topup_method, $4, now() + make_interval(secs => $5)) RETURNING id, created_at",
    )
    .bind(wallet)
    .bind(&trade_no)
    .bind(method)
    .bind(amount)
    .bind(ttl.num_seconds() as f64)
    .fetch_one(&mut *tx)
    .await?;
    let invoice_id: Uuid = sqlx::query_scalar(
        "INSERT INTO invoices (source_type, source_id, amount, carrier_type, carrier_id, donate_code, buyer_tax_id, buyer_name)
         VALUES ('topup', $1, $2, $3::invoice_carrier_type, $4, $5, $6, $7) RETURNING id",
    )
    .bind(id)
    .bind(amount)
    .bind(&inv.carrier_type)
    .bind(&inv.carrier_id)
    .bind(&inv.donate_code)
    .bind(&inv.buyer_tax_id)
    .bind(&inv.buyer_name)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query("UPDATE topups SET invoice_id = $2 WHERE id = $1")
        .bind(id)
        .bind(invoice_id)
        .execute(&mut *tx)
        .await?;

    let out = json!({
        "topup": { "id": id, "amount": amount, "method": method, "provider": "ecpay", "status": "pending",
                   "paid_at": null, "created_at": created_at },
        "payment": payment_form(&cfg, &trade_no, amount, method, created_at),
    });
    idempotency::finish(&mut tx, &scope, key, 201, &out).await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        [(header::CACHE_CONTROL, "private, no-store")],
        Json(out),
    )
        .into_response())
}

// ---- POST /webhooks/payments/{provider} ----

fn ok_text() -> Response {
    (
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        "1|OK",
    )
        .into_response()
}

fn provider_gate(provider: &str) -> Result<Ecpay, AppError> {
    match provider {
        "ecpay" => ecpay(),
        "tappay" => Err(AppError::problem(
            501,
            "PROVIDER_NOT_IMPLEMENTED",
            "tappay 尚未實作",
        )),
        _ => Err(AppError::NotFound),
    }
}

/// 解析表單並以常數時間驗 CheckMacValue。
fn verified_form(
    form: Result<Form<Vec<(String, String)>>, FormRejection>,
    cfg: &Ecpay,
) -> Result<Vec<(String, String)>, AppError> {
    let Form(params) =
        form.map_err(|_| AppError::problem(400, "BAD_REQUEST", "請求內容必須是表單"))?;
    let given = param(&params, "CheckMacValue")
        .unwrap_or_default()
        .to_uppercase();
    if !bool::from(given.as_bytes().ct_eq(check_mac(&params, cfg).as_bytes())) {
        return Err(AppError::problem(401, "UNAUTHORIZED", "簽章錯誤"));
    }
    Ok(params)
}

fn param<'a>(params: &'a [(String, String)], k: &str) -> Option<&'a str> {
    params.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str())
}

/// 驗簽 → 讀訂單（無鎖）→ 核對通知金額 → QueryTradeInfo 二次確認（交易外）→ 開交易 FOR UPDATE 重讀後入帳（冪等）。
/// 成功一律回純文字 `1|OK`，否則綠界會重送。唯讀模式由全域 read_only middleware 回 503（供應商稍後重送）。
async fn webhook(
    State(st): State<AppState>,
    Path(provider): Path<String>,
    form: Result<Form<Vec<(String, String)>>, FormRejection>,
) -> Result<Response, AppError> {
    let cfg = provider_gate(&provider)?;
    let params = verified_form(form, &cfg)?;
    let get = |k: &str| param(&params, k);
    let trade_no = get("MerchantTradeNo").unwrap_or_default();
    let bad = || AppError::problem(400, "BAD_REQUEST", "通知欄位不完整");

    let row: Option<(Uuid, i64, String)> = sqlx::query_as(
        "SELECT id, amount, status::text FROM topups WHERE merchant_trade_no = $1 AND provider = 'ecpay'",
    )
    .bind(trade_no)
    .fetch_optional(&st.pool)
    .await?;
    let Some((id, amount, status)) = row else {
        return Err(AppError::problem(404, "NOT_FOUND", "查無儲值訂單"));
    };
    // 已入帳、或已進入退款 / 爭議流程：重送一律冪等
    if !matches!(status.as_str(), "pending" | "expired" | "failed") {
        return Ok(ok_text());
    }
    if get("RtnCode") != Some("1") {
        // 條件式 UPDATE 本身即原子：與入帳競爭時，已 paid 的列不會被改回 failed
        sqlx::query("UPDATE topups SET status = 'failed', failure_reason = $2, updated_at = now() WHERE id = $1 AND status = 'pending'")
            .bind(id)
            .bind(get("RtnMsg"))
            .execute(&st.pool)
            .await?;
        return Ok(ok_text());
    }
    let txn = get("TradeNo").filter(|t| !t.is_empty()).ok_or_else(bad)?;
    // 金額不符一律不入帳、寫 audit_logs 並告警；已逾期但實際付款成功仍入帳，錢已收
    if get("TradeAmt").and_then(|a| a.parse::<i64>().ok()) != Some(amount) {
        audit(
            &st.pool,
            "topup.amount_mismatch",
            id,
            json!({ "expected": amount, "notified": get("TradeAmt"), "trade_no": txn }),
        )
        .await?;
        tracing::error!(topup = %id, "topup 通知金額與訂單不符，未入帳");
        return Err(AppError::problem(
            422,
            "AMOUNT_MISMATCH",
            "通知金額與訂單不符",
        ));
    }
    // 外部 HTTP 絕不在持有 topups 列鎖的交易內執行（會佔住連線與鎖）
    confirm(&st.pool, &cfg, id, trade_no, amount, txn).await?;
    credit(&st.pool, id, txn).await?;
    Ok(ok_text())
}

async fn audit(pool: &PgPool, action: &str, id: Uuid, diff: Value) -> Result<(), AppError> {
    sqlx::query("INSERT INTO audit_logs (actor_type, action, entity, entity_id, diff) VALUES ('system', $1, 'topups', $2, $3)")
        .bind(action)
        .bind(id)
        .bind(diff)
        .execute(pool)
        .await?;
    Ok(())
}

/// 綠界 QueryTradeInfo/V5（POST form），回應為 URL-encoded 字串。逾時 / 網路錯誤 / 非 2xx 一律 503，讓綠界稍後重送。
async fn query_trade(cfg: &Ecpay, trade_no: &str) -> Result<Vec<(String, String)>, AppError> {
    let unavailable = |why: &str| {
        tracing::error!(trade_no, why, "綠界 QueryTradeInfo 失敗");
        AppError::problem(
            503,
            "PAYMENT_QUERY_FAILED",
            "無法向金流商確認交易，請稍後重送",
        )
    };
    let mut f: Vec<(String, String)> = vec![
        ("MerchantID".into(), cfg.merchant_id.clone()),
        ("MerchantTradeNo".into(), trade_no.into()),
        ("TimeStamp".into(), Utc::now().timestamp().to_string()),
    ];
    let mac = check_mac(&f, cfg);
    f.push(("CheckMacValue".into(), mac));
    let res = reqwest::Client::new()
        .post(&cfg.query_url)
        .form(&f)
        .timeout(std::time::Duration::from_secs(cfg.query_timeout_secs))
        .send()
        .await
        .map_err(|e| unavailable(&e.to_string()))?;
    if !res.status().is_success() {
        return Err(unavailable(res.status().as_str()));
    }
    let text = res.text().await.map_err(|e| unavailable(&e.to_string()))?;
    serde_urlencoded::from_str(&text).map_err(|e| unavailable(&e.to_string()))
}

/// 入帳前二次確認：回應簽章正確，且 TradeStatus=1（已付款）、金額、MerchantTradeNo、TradeNo 皆與訂單一致。
/// 任一不符 → 不入帳、寫 audit_logs（topup.query_mismatch）並告警，回 422 讓綠界重送。
async fn confirm(
    pool: &PgPool,
    cfg: &Ecpay,
    id: Uuid,
    trade_no: &str,
    amount: i64,
    txn: &str,
) -> Result<(), AppError> {
    let q = query_trade(cfg, trade_no).await?;
    let get = |k: &str| param(&q, k);
    let given = get("CheckMacValue").unwrap_or_default().to_uppercase();
    let sig_ok = bool::from(given.as_bytes().ct_eq(check_mac(&q, cfg).as_bytes()));
    let paid = get("TradeStatus") == Some("1")
        && get("TradeAmt").and_then(|a| a.parse::<i64>().ok()) == Some(amount)
        && get("MerchantTradeNo") == Some(trade_no)
        && get("TradeNo") == Some(txn);
    if !sig_ok || !paid {
        audit(
            pool,
            "topup.query_mismatch",
            id,
            json!({
                "expected": amount, "trade_no": txn, "signature_ok": sig_ok,
                "queried": { "status": get("TradeStatus"), "amount": get("TradeAmt"),
                             "merchant_trade_no": get("MerchantTradeNo"), "trade_no": get("TradeNo") },
            }),
        )
        .await?;
        tracing::error!(topup = %id, signature_ok = sig_ok, "topup 二次確認與通知不符，未入帳");
        return Err(AppError::problem(
            422,
            "PAYMENT_QUERY_MISMATCH",
            "金流商查詢結果與通知不符",
        ));
    }
    Ok(())
}

/// 入帳交易：FOR UPDATE 重讀訂單（確認期間可能已被並行通知入帳或其他流程改變狀態），已入帳則冪等略過。
async fn credit(pool: &PgPool, id: Uuid, txn: &str) -> Result<(), AppError> {
    let mut tx = pool.begin().await?;
    let (wallet, amount, status): (Uuid, i64, String) = sqlx::query_as(
        "SELECT wallet_id, amount, status::text FROM topups WHERE id = $1 FOR UPDATE",
    )
    .bind(id)
    .fetch_one(&mut *tx)
    .await?;
    if !matches!(status.as_str(), "pending" | "expired" | "failed") {
        return Ok(());
    }
    points::lock_wallets(&mut tx, &[wallet]).await?;
    sqlx::query("UPDATE topups SET status = 'paid', paid_at = now(), provider_txn_id = $2, failure_reason = NULL, updated_at = now() WHERE id = $1")
        .bind(id)
        .bind(txn)
        .execute(&mut *tx)
        .await?;
    points::post(
        &mut tx,
        wallet,
        amount,
        Entry {
            kind: "topup",
            ref_type: "topup",
            ref_id: Some(id),
            note: None,
            actor: None,
        },
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

// ---- POST /webhooks/payments/{provider}/dispute（接口位置，D57）----

/// 綠界尚未提供可驗證的爭議通知格式（拒付多半走銀行 / 特店後台）；取得官方格式後在此驗簽並呼叫
/// `wallet_ops::dispute_open / charge_back / dispute_won`。目前拒付由營運入口 `POST /admin/topups/{id}/dispute` 處理。
async fn webhook_dispute() -> Result<Response, AppError> {
    Err(AppError::problem(
        501,
        "DISPUTE_WEBHOOK_NOT_AVAILABLE",
        "尚未提供可驗證的爭議通知格式，請由營運後台處理",
    ))
}

// ---- POST /webhooks/payments/{provider}/info（PaymentInfoURL 取號通知）----

/// 綠界 ExpireDate：CVS / BARCODE 為 `yyyy/MM/dd HH:mm:ss`，ATM 為 `yyyy/MM/dd`（以官方文件為準），皆為台灣時間；日期型取當日 23:59:59。
fn parse_expire(s: &str) -> Option<DateTime<Utc>> {
    let tw = FixedOffset::east_opt(8 * 3600)?;
    let naive = NaiveDateTime::parse_from_str(s.trim(), "%Y/%m/%d %H:%M:%S")
        .ok()
        .or_else(|| {
            NaiveDate::parse_from_str(s.trim(), "%Y/%m/%d")
                .ok()?
                .and_hms_opt(23, 59, 59)
        })?;
    Some(tw.from_local_datetime(&naive).single()?.with_timezone(&Utc))
}

/// 取號成功通知：驗簽 → 核對金額 → 寫入 payment_info（僅 pending；重送覆寫為相同內容無害）→ `1|OK`。
/// RtnCode 依付款方式不同（ATM=2、CVS / BARCODE=10100073，以官方文件為準），故不比對特定碼：
/// 只排除 `1`（那是付款通知），其餘以「該付款方式必要的取號欄位是否齊全」判斷，欄位不齊時記 warn 並回 OK（重送也不會補齊）。
async fn webhook_info(
    State(st): State<AppState>,
    Path(provider): Path<String>,
    form: Result<Form<Vec<(String, String)>>, FormRejection>,
) -> Result<Response, AppError> {
    let cfg = provider_gate(&provider)?;
    let params = verified_form(form, &cfg)?;
    let get = |k: &str| param(&params, k).filter(|v| !v.is_empty());
    let trade_no = get("MerchantTradeNo").unwrap_or_default();
    let row: Option<(Uuid, i64, String, String, DateTime<Utc>)> = sqlx::query_as(
        "SELECT id, amount, method::text, status::text, expires_at FROM topups WHERE merchant_trade_no = $1 AND provider = 'ecpay'",
    )
    .bind(trade_no)
    .fetch_optional(&st.pool)
    .await?;
    let Some((id, amount, method, status, expires_at)) = row else {
        return Err(AppError::problem(404, "NOT_FOUND", "查無儲值訂單"));
    };
    if status != "pending" || get("RtnCode") == Some("1") {
        return Ok(ok_text());
    }
    if get("TradeAmt").and_then(|a| a.parse::<i64>().ok()) != Some(amount) {
        audit(
            &st.pool,
            "topup.info_amount_mismatch",
            id,
            json!({ "expected": amount, "notified": get("TradeAmt") }),
        )
        .await?;
        tracing::error!(topup = %id, "topup 取號通知金額與訂單不符");
        return Err(AppError::problem(
            422,
            "AMOUNT_MISMATCH",
            "通知金額與訂單不符",
        ));
    }
    let expire_at = get("ExpireDate")
        .and_then(parse_expire)
        .unwrap_or(expires_at)
        .to_rfc3339_opts(SecondsFormat::Secs, true);
    let info = match method.as_str() {
        "cvs_code" => get("PaymentNo")
            .map(|n| json!({ "type": method, "payment_no": n, "expire_at": expire_at })),
        "cvs_barcode" => match (get("Barcode1"), get("Barcode2"), get("Barcode3")) {
            (Some(a), Some(b), Some(c)) => Some(
                json!({ "type": method, "barcode1": a, "barcode2": b, "barcode3": c, "expire_at": expire_at }),
            ),
            _ => None,
        },
        "atm" => match (get("BankCode"), get("vAccount")) {
            (Some(bank), Some(acct)) => Some(
                json!({ "type": method, "bank_code": bank, "account": acct, "expire_at": expire_at }),
            ),
            _ => None,
        },
        _ => None,
    };
    let Some(info) = info else {
        tracing::warn!(topup = %id, method, "取號通知缺少必要欄位或付款方式不適用，略過");
        return Ok(ok_text());
    };
    sqlx::query("UPDATE topups SET payment_info = $2, updated_at = now() WHERE id = $1 AND status = 'pending'")
        .bind(id)
        .bind(info)
        .execute(&st.pool)
        .await?;
    Ok(ok_text())
}

// ---- GET /wallet/topups、GET /wallet/topups/{id} ----

type TopupRow = (
    Uuid,
    i64,
    String,
    String,
    DateTime<Utc>,
    Option<DateTime<Utc>>,
    DateTime<Utc>,
    Option<Value>,
);
const TOPUP_COLS: &str = "t.id, t.amount, t.method::text, t.status::text, t.created_at, t.paid_at, t.expires_at, t.payment_info";

fn topup_json(r: TopupRow) -> Value {
    json!({ "id": r.0, "amount": r.1, "method": r.2, "status": r.3, "created_at": r.4,
            "paid_at": r.5, "expires_at": r.6, "payment_info": r.7 })
}

fn no_store(body: Value) -> Response {
    ([(header::CACHE_CONTROL, "private, no-store")], Json(body)).into_response()
}

#[derive(serde::Deserialize)]
struct ListQuery {
    limit: Option<i64>,
}

/// 本人的儲值單，最新在前；limit 預設 20、上限 50。
async fn list(
    State(st): State<AppState>,
    user: CurrentUser,
    Query(q): Query<ListQuery>,
) -> Result<Response, AppError> {
    let rows: Vec<TopupRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {TOPUP_COLS} FROM topups t JOIN point_wallets w ON w.id = t.wallet_id
          WHERE w.user_id = $1 ORDER BY t.created_at DESC, t.id DESC LIMIT $2"
    )))
    .bind(user.id)
    .bind(q.limit.unwrap_or(20).clamp(1, 50))
    .fetch_all(&st.pool)
    .await?;
    Ok(no_store(
        json!({ "topups": rows.into_iter().map(topup_json).collect::<Vec<_>>() }),
    ))
}

/// 非本人或不存在一律 404，不洩漏存在性。
async fn detail(
    State(st): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<Response, AppError> {
    let row: Option<TopupRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {TOPUP_COLS} FROM topups t JOIN point_wallets w ON w.id = t.wallet_id
          WHERE t.id = $1 AND w.user_id = $2"
    )))
    .bind(id)
    .bind(user.id)
    .fetch_optional(&st.pool)
    .await?;
    let row = row.ok_or(AppError::NotFound)?;
    Ok(no_store(json!({ "topup": topup_json(row) })))
}
