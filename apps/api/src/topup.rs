//! 儲值（骨架，金流商尚未接入）：POST /topups 建立待付款訂單；POST /webhooks/topup/{provider} 驗簽後入帳。
//! 入帳只走 `points::post`（kind=topup），同交易把 topups 標為 paid；重送通知冪等（已 paid 直接回 200）。
//! 功能預設關閉：未設 TOPUP_WEBHOOK_SECRET（≥32 字元）時兩個端點都回 503。
//! 接金流商時只改兩處：`verify`（換成該商的簽章，例如綠界 CheckMacValue）與 `Notice` 的欄位對應；
//! 另須在 create 回傳 payment_url，並以金流商查詢 API 二次確認金額 / 狀態（見 docs 儲值評估）。
use crate::{
    config,
    error::AppError,
    points::{self, Entry},
    session::CurrentUser,
    AppState,
};
use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::post,
    Json, Router,
};
use hmac::{Hmac, Mac};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::Sha256;
use uuid::Uuid;

/// 可選面額（NT$）。
pub const DENOMINATIONS: [i64; 4] = [500, 1000, 2000, 5000];
/// 單一錢包餘額（含待付款）上限：電子支付機構管理條例的儲值上限 NT$50,000。
pub const WALLET_CAP: i64 = 50_000;
const MAX_PENDING: i64 = 5;
const PROVIDER: &str = "ecpay";
const METHODS: [&str; 4] = ["card", "cvs_code", "cvs_barcode", "atm"];

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/topups", post(create))
        .route("/webhooks/topup/{provider}", post(webhook))
}

fn secret() -> Result<String, AppError> {
    config::get()
        .topup_webhook_secret
        .ok_or_else(|| AppError::problem(503, "TOPUP_DISABLED", "儲值功能尚未開放"))
}

#[derive(Deserialize)]
struct NewTopup {
    amount: i64,
    method: Option<String>,
}

/// POST /topups：建立待付款儲值訂單（登入必填）。payment_url 在金流商接入前為 null。
async fn create(
    State(st): State<AppState>,
    user: CurrentUser,
    Json(b): Json<NewTopup>,
) -> Result<(StatusCode, Json<Value>), AppError> {
    secret()?;
    if !DENOMINATIONS.contains(&b.amount) {
        return Err(AppError::invalid(
            "/amount",
            "ENUM",
            "面額必須是 500 / 1000 / 2000 / 5000",
        ));
    }
    let method = b.method.as_deref().unwrap_or("card");
    if !METHODS.contains(&method) {
        return Err(AppError::invalid("/method", "ENUM", "付款方式不支援"));
    }
    let mut tx = st.pool.begin().await?;
    let wallet = points::ensure_wallet(&mut tx, user.id).await?;
    points::lock_wallets(&mut tx, &[wallet]).await?; // 同使用者的建立請求序列化，上限檢查才不會被併發繞過
    let (bal, pending, pending_n): (i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT balance FROM point_wallets WHERE id = $1),
                COALESCE(SUM(amount), 0)::bigint, COUNT(*)
           FROM topups WHERE user_id = $2 AND status = 'pending' AND expires_at > now()",
    )
    .bind(wallet)
    .bind(user.id)
    .fetch_one(&mut *tx)
    .await?;
    if pending_n >= MAX_PENDING {
        return Err(AppError::problem(
            429,
            "TOO_MANY_PENDING_TOPUPS",
            "待付款的儲值訂單太多，請先完成或等待過期",
        ));
    }
    if bal + pending + b.amount > WALLET_CAP {
        return Err(AppError::problem(
            409,
            "WALLET_CAP_EXCEEDED",
            format!("儲值後餘額將超過上限 {WALLET_CAP} 點"),
        ));
    }
    let trade_no = format!("WL{}", &Uuid::new_v4().simple().to_string()[..18]); // 綠界訂單編號上限 20 字元
    let ttl_min = if method == "card" { 30 } else { 3 * 24 * 60 };
    let (id, expires_at): (Uuid, chrono::DateTime<chrono::Utc>) = sqlx::query_as(
        "INSERT INTO topups (user_id, merchant_trade_no, provider, method, amount, expires_at)
         VALUES ($1, $2, $3, $4, $5, now() + make_interval(mins => $6)) RETURNING id, expires_at",
    )
    .bind(user.id)
    .bind(&trade_no)
    .bind(PROVIDER)
    .bind(method)
    .bind(b.amount)
    .bind(ttl_min)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "id": id, "merchant_trade_no": trade_no, "amount": b.amount, "method": method,
            "status": "pending", "expires_at": expires_at, "payment_url": null
        })),
    ))
}

/// 通用簽章：HMAC-SHA256(secret, 原始 body) 的 hex，放 X-Topup-Signature。接金流商時換成該商的驗簽。
fn verify(headers: &HeaderMap, body: &[u8], secret: &str) -> bool {
    let Some(sig) = headers
        .get("x-topup-signature")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| hex::decode(s).ok())
    else {
        return false;
    };
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC 接受任意長度金鑰");
    mac.update(body);
    mac.verify_slice(&sig).is_ok() // 常數時間比較
}

#[derive(Deserialize)]
struct Notice {
    merchant_trade_no: String,
    provider_trade_no: String,
    amount: i64,
    status: String, // paid | failed
}

/// POST /webhooks/topup/{provider}：驗簽 → 鎖訂單 → 核對金額 → 入帳（冪等）。
async fn webhook(
    State(st): State<AppState>,
    Path(provider): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Value>, AppError> {
    if !verify(&headers, &body, &secret()?) {
        return Err(AppError::problem(401, "BAD_SIGNATURE", "簽章錯誤"));
    }
    let n: Notice = serde_json::from_slice(&body)
        .map_err(|_| AppError::problem(400, "BAD_REQUEST", "請求內容必須是 JSON"))?;
    let mut tx = st.pool.begin().await?;
    let row: Option<(Uuid, Uuid, i64, String)> = sqlx::query_as(
        "SELECT id, user_id, amount, status FROM topups
          WHERE merchant_trade_no = $1 AND provider = $2 FOR UPDATE",
    )
    .bind(&n.merchant_trade_no)
    .bind(&provider)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((id, user_id, amount, status)) = row else {
        return Err(AppError::problem(404, "TOPUP_NOT_FOUND", "查無儲值訂單"));
    };
    if status == "paid" {
        return Ok(Json(json!({ "ok": true, "replayed": true })));
    }
    if n.status != "paid" {
        sqlx::query("UPDATE topups SET status = 'failed' WHERE id = $1 AND status = 'pending'")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        return Ok(Json(json!({ "ok": true })));
    }
    // 金額不符一律不入帳（留 pending 待人工 / 對帳處理）；已逾期但實際付款成功仍入帳，錢已收
    if n.amount != amount {
        return Err(AppError::problem(
            422,
            "AMOUNT_MISMATCH",
            "通知金額與訂單不符",
        ));
    }
    let wallet = points::ensure_wallet(&mut tx, user_id).await?;
    points::lock_wallets(&mut tx, &[wallet]).await?;
    sqlx::query(
        "UPDATE topups SET status = 'paid', paid_at = now(), provider_trade_no = $2 WHERE id = $1",
    )
    .bind(id)
    .bind(&n.provider_trade_no)
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
    Ok(Json(json!({ "ok": true })))
}
