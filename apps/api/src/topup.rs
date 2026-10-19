//! 儲值（docs/04 `POST /wallet/topups`、`POST /webhooks/payments/{provider}`，D53 / D54）。
//! 1 點 = NT$1。目前只實作 provider=ecpay（綠界 AioCheckOut，form_post）；tappay 與 line_pay / apple_pay 尚未實作。
//! 建單：Idempotency-Key 必填，占位與寫入同一交易；入帳只在 webhook，且只走 `points::post`（kind=topup）。
//! 憑證（ECPAY_*）未設齊時兩個端點都回 503 TOPUP_DISABLED。
//! 未做：綠界 QueryTradeInfo 二次確認、PaymentInfoURL（超商代碼 / ATM 取號通知）、拒付 / 爭議、自動儲值。
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
    extract::{rejection::FormRejection, Form, Path, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use chrono::{DateTime, Duration, FixedOffset, Utc};
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
        .route("/wallet/topups", post(create))
        .route("/webhooks/payments/{provider}", post(webhook))
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
        ("ClientBackURL".into(), config::get().app_url),
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
        return Err(AppError::problem(
            409,
            "WALLET_CAP_EXCEEDED",
            format!("儲值後餘額將超過上限 {WALLET_CAP} 點"),
        ));
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

/// 驗簽 → 鎖訂單 → 核對金額 → 入帳（冪等）。成功一律回純文字 `1|OK`，否則綠界會重送。
/// 唯讀模式由全域 read_only middleware 回 503（供應商稍後重送）。
async fn webhook(
    State(st): State<AppState>,
    Path(provider): Path<String>,
    form: Result<Form<Vec<(String, String)>>, FormRejection>,
) -> Result<Response, AppError> {
    match provider.as_str() {
        "ecpay" => {}
        "tappay" => {
            return Err(AppError::problem(
                501,
                "PROVIDER_NOT_IMPLEMENTED",
                "tappay 尚未實作",
            ))
        }
        _ => return Err(AppError::NotFound),
    }
    let cfg = ecpay()?;
    let Form(params) =
        form.map_err(|_| AppError::problem(400, "BAD_REQUEST", "請求內容必須是表單"))?;
    let get = |k: &str| params.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
    let given = get("CheckMacValue").unwrap_or_default().to_uppercase();
    if !bool::from(given.as_bytes().ct_eq(check_mac(&params, &cfg).as_bytes())) {
        return Err(AppError::problem(401, "UNAUTHORIZED", "簽章錯誤"));
    }
    let trade_no = get("MerchantTradeNo").unwrap_or_default();
    let bad = || AppError::problem(400, "BAD_REQUEST", "通知欄位不完整");
    let provider_txn = get("TradeNo").filter(|t| !t.is_empty());

    let mut tx = st.pool.begin().await?;
    let row: Option<(Uuid, Uuid, i64, String)> = sqlx::query_as(
        "SELECT id, wallet_id, amount, status::text FROM topups
          WHERE merchant_trade_no = $1 AND provider = 'ecpay' FOR UPDATE",
    )
    .bind(trade_no)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((id, wallet, amount, status)) = row else {
        return Err(AppError::problem(404, "NOT_FOUND", "查無儲值訂單"));
    };
    // 已入帳、或已進入退款 / 爭議流程：重送一律冪等
    if !matches!(status.as_str(), "pending" | "expired" | "failed") {
        return Ok(ok_text());
    }
    if get("RtnCode") != Some("1") {
        sqlx::query("UPDATE topups SET status = 'failed', failure_reason = $2, updated_at = now() WHERE id = $1 AND status = 'pending'")
            .bind(id)
            .bind(get("RtnMsg"))
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        return Ok(ok_text());
    }
    let txn = provider_txn.ok_or_else(bad)?;
    // 金額不符一律不入帳、寫 audit_logs 並告警；已逾期但實際付款成功仍入帳，錢已收
    if get("TradeAmt").and_then(|a| a.parse::<i64>().ok()) != Some(amount) {
        sqlx::query("INSERT INTO audit_logs (actor_type, action, entity, entity_id, diff) VALUES ('system', 'topup.amount_mismatch', 'topups', $1, $2)")
            .bind(id)
            .bind(json!({ "expected": amount, "notified": get("TradeAmt"), "trade_no": txn }))
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        tracing::error!(topup = %id, "topup 通知金額與訂單不符，未入帳");
        return Err(AppError::problem(
            422,
            "AMOUNT_MISMATCH",
            "通知金額與訂單不符",
        ));
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
    Ok(ok_text())
}
