//! 錢包後續功能（D55）：現金退款申請與營運處理、拒付處理（4.3s）、欠款結案、自動儲值設定。
//! 鎖序：點數異動一律先鎖錢包（`points::lock_wallets` / `FOR UPDATE`），lots 不另鎖。
//! 拒付函式（`dispute_open` / `charge_back` / `dispute_won`）是給金流 webhook 呼叫的 library 函式：
//! 呼叫端在自己的交易內傳入 `&mut Tx`；本模組先鎖 topups 列、再鎖錢包（與 topup.rs webhook 的鎖序一致）。
//! 營運入口（D57）：`POST /admin/topups/{id}/dispute` 在單一交易內呼叫上述函式並寫 audit_logs；供應商 webhook 待取得官方爭議格式。
//! 自動儲值只有設定 CRUD；觸發 job 與扣款需要金流授權，尚未實作。
use crate::{
    admin::{audit, limit, page, PageQ, Staff},
    error::AppError,
    idempotency, points,
    points::Tx,
    ratelimit,
    session::CurrentUser,
    AppState,
};
use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, patch, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/wallet/cash-refunds",
            post(cash_refund_create).get(cash_refund_mine),
        )
        .route("/admin/topups", get(topup_list))
        .route("/admin/topups/{id}/dispute", post(topup_dispute))
        .route("/admin/cash-refunds", get(cash_refund_list))
        .route("/admin/cash-refunds/{id}", patch(cash_refund_patch))
        .route("/admin/wallet-debts", get(debt_list))
        .route("/admin/wallet-debts/{id}", patch(debt_patch))
        .route(
            "/wallet/auto-topup",
            get(auto_get).put(auto_put).delete(auto_delete),
        )
}

type R<T> = Result<T, AppError>;

fn json_resp(status: u16, body: &Value, replay: bool) -> Response {
    let mut b = Response::builder()
        .status(StatusCode::from_u16(status).unwrap())
        .header(header::CACHE_CONTROL, "private, no-store")
        .header(header::CONTENT_TYPE, "application/json");
    if replay {
        b = b.header("Idempotency-Replayed", HeaderValue::from_static("true"));
    }
    b.body(axum::body::Body::from(body.to_string())).unwrap()
}

fn parse(b: &[u8]) -> R<Value> {
    serde_json::from_slice(b).map_err(|_| AppError::invalid("/", "INVALID", "請求內容格式錯誤"))
}

fn text_field(b: &Value, key: &str, required: bool) -> R<Option<String>> {
    match b.get(key).and_then(Value::as_str) {
        Some(s) => {
            let s = crate::validate::text(s, &format!("/{key}"), false)?;
            if s.is_empty() || s.chars().count() > 200 {
                return Err(AppError::invalid(
                    &format!("/{key}"),
                    "RANGE",
                    &format!("{key} 需為 1–200 字"),
                ));
            }
            Ok(Some(s))
        }
        None if required => Err(AppError::invalid(
            &format!("/{key}"),
            "REQUIRED",
            &format!("{key} 必填"),
        )),
        None => Ok(None),
    }
}

// ================= 現金退款 =================

/// POST /wallet/cash-refunds（4.3h）：先從餘額扣點（只扣 cash_refundable 批次）並建立申請。Idempotency-Key 必填。
async fn cash_refund_create(
    State(st): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    body: Bytes,
) -> R<Response> {
    let key = idempotency::key(&headers)?;
    let b = parse(&body)?;
    let amount = b
        .get("amount")
        .and_then(Value::as_i64)
        .filter(|a| (1..=50_000).contains(a))
        .ok_or_else(|| AppError::invalid("/amount", "RANGE", "amount 必須介於 1 與 50000"))?;
    let scope = format!("user:{}:POST /wallet/cash-refunds", user.id);
    let hash = idempotency::request_hash("POST /wallet/cash-refunds", &body);
    let mut tx = st.pool.begin().await?;
    if let idempotency::Begin::Replay(s, b) =
        idempotency::begin(&mut tx, &scope, key, &hash).await?
    {
        return Ok(json_resp(s as u16, &b, true));
    }
    ratelimit::check(&mut *tx, &format!("cash_refund:{}", user.id), 10, 3600).await?;
    let wallet = points::ensure_wallet(&mut tx, user.id).await?;
    let status: String =
        sqlx::query_scalar("SELECT status::text FROM point_wallets WHERE id = $1 FOR UPDATE")
            .bind(wallet)
            .fetch_one(&mut *tx)
            .await?;
    if status == "frozen" {
        return Err(AppError::problem(
            403,
            "WALLET_FROZEN",
            "錢包已凍結，無法申請現金退款，請聯絡客服",
        ));
    }
    let rid: Uuid = sqlx::query_scalar(
        "INSERT INTO cash_refund_requests (wallet_id, amount) VALUES ($1, $2) RETURNING id",
    )
    .bind(wallet)
    .bind(amount)
    .fetch_one(&mut *tx)
    .await?;
    let allocs = points::cash_refund_debit(&mut tx, wallet, amount, rid).await?;
    let alloc_json: Vec<Value> = allocs
        .iter()
        .map(|(l, p)| json!({ "lot_id": l, "points": p }))
        .collect();
    let (created_at, balance): (chrono::DateTime<chrono::Utc>, i64) = sqlx::query_as(
        "UPDATE cash_refund_requests SET lot_allocations = $2 WHERE id = $1
         RETURNING created_at, (SELECT balance FROM point_wallets WHERE id = $3)",
    )
    .bind(rid)
    .bind(json!(alloc_json))
    .bind(wallet)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query("INSERT INTO audit_logs (actor_type, actor_id, action, entity, entity_id, diff) VALUES ('user', $1, 'cash_refund.request', 'cash_refund_requests', $2, $3)")
        .bind(user.id).bind(rid).bind(json!({ "amount": amount, "lot_allocations": alloc_json })).execute(&mut *tx).await?;
    let resp = json!({
        "request": { "id": rid, "amount": amount, "status": "requested", "created_at": created_at, "completed_at": null },
        "wallet": { "balance": balance },
    });
    idempotency::finish(&mut tx, &scope, key, 201, &resp).await?;
    tx.commit().await?;
    Ok(json_resp(201, &resp, false))
}

/// GET /wallet/cash-refunds?limit=：本人的現金退款申請，最新在前；limit 預設 20、上限 50。不回 lot_allocations / 收款資訊。
async fn cash_refund_mine(
    State(st): State<AppState>,
    user: CurrentUser,
    Query(q): Query<PageQ>,
) -> R<Response> {
    let rows: Vec<Value> = sqlx::query_scalar(
        "SELECT jsonb_build_object('id', r.id, 'amount', r.amount, 'status', r.status, 'created_at', r.created_at,
                'completed_at', r.completed_at, 'reject_reason', r.reject_reason)
           FROM cash_refund_requests r JOIN point_wallets w ON w.id = r.wallet_id
          WHERE w.user_id = $1 ORDER BY r.created_at DESC, r.id DESC LIMIT $2")
        .bind(user.id).bind(q.limit.unwrap_or(20).clamp(1, 50)).fetch_all(&st.pool).await?;
    Ok(json_resp(200, &json!({ "requests": rows }), false))
}

/// GET /admin/cash-refunds?status=（requested | processing | completed | rejected）
async fn cash_refund_list(
    State(st): State<AppState>,
    Staff(_): Staff,
    Query(q): Query<PageQ>,
) -> R<Response> {
    if q.status
        .as_deref()
        .is_some_and(|s| !["requested", "processing", "completed", "rejected"].contains(&s))
    {
        return Err(AppError::invalid("/status", "ENUM", "status 不正確"));
    }
    let rows: Vec<(Uuid, Value)> = sqlx::query_as(
        "SELECT r.id, jsonb_build_object('id', r.id, 'amount', r.amount, 'status', r.status,
                'user', jsonb_build_object('id', u.id, 'display_name', u.display_name, 'email', u.email),
                'lot_allocations', r.lot_allocations, 'reject_reason', r.reject_reason, 'processed_by', r.processed_by,
                'created_at', r.created_at, 'completed_at', r.completed_at)
           FROM cash_refund_requests r JOIN point_wallets w ON w.id = r.wallet_id JOIN users u ON u.id = w.user_id
          WHERE ($1::text IS NULL OR r.status::text = $1) AND ($2::uuid IS NULL OR r.id > $2)
          ORDER BY r.id LIMIT $3")
        .bind(&q.status).bind(q.cursor).bind(limit(&q) + 1).fetch_all(&st.pool).await?;
    Ok(page(rows, limit(&q)))
}

fn bad_transition() -> AppError {
    AppError::problem(409, "INVALID_STATE_TRANSITION", "此狀態無法這樣變更")
}

/// PATCH /admin/cash-refunds/{id}（4.3i）：requested → processing → completed；requested / processing → rejected（reject_reason 必填，
/// 依 lot_allocations 退回原批次）。先鎖錢包再改申請，狀態守衛在 UPDATE 的 WHERE，保證只退一次。
async fn cash_refund_patch(
    State(st): State<AppState>,
    Staff(sid): Staff,
    Path(id): Path<Uuid>,
    Json(b): Json<Value>,
) -> R<Json<Value>> {
    let to = b
        .get("status")
        .and_then(Value::as_str)
        .filter(|s| ["processing", "completed", "rejected"].contains(s))
        .ok_or_else(|| {
            AppError::invalid(
                "/status",
                "ENUM",
                "status 必須是 processing | completed | rejected",
            )
        })?;
    let reason = text_field(&b, "reject_reason", to == "rejected")?;
    let mut tx = st.pool.begin().await?;
    let wallet: Uuid =
        sqlx::query_scalar("SELECT wallet_id FROM cash_refund_requests WHERE id = $1")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(AppError::NotFound)?;
    points::lock_wallets(&mut tx, &[wallet]).await?;
    let row: Option<(i64, Value, String)> = sqlx::query_as(
        "UPDATE cash_refund_requests SET status = $2::text::refund_request_status, processed_by = $3,
                reject_reason = COALESCE($4, reject_reason),
                completed_at = CASE WHEN $2::text IN ('completed', 'rejected') THEN now() ELSE completed_at END
          WHERE id = $1 AND CASE $2::text WHEN 'processing' THEN status = 'requested'
                                    WHEN 'completed' THEN status = 'processing'
                                    ELSE status IN ('requested', 'processing') END
          RETURNING amount, lot_allocations, status::text")
        .bind(id).bind(to).bind(sid).bind(&reason).fetch_optional(&mut *tx).await?;
    let (amount, allocs, _) = row.ok_or_else(bad_transition)?;
    if to == "rejected" {
        let allocs: Vec<(Uuid, i64)> = allocs
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|e| {
                        Some((e["lot_id"].as_str()?.parse().ok()?, e["points"].as_i64()?))
                    })
                    .collect()
            })
            .unwrap_or_default();
        points::cash_refund_restore(&mut tx, wallet, id, &allocs, sid).await?;
    }
    audit(
        &mut *tx,
        sid,
        "cash_refund.process",
        "cash_refund_requests",
        Some(id),
        json!({ "status": to, "amount": amount, "reject_reason": reason }),
    )
    .await?;
    if to != "processing" {
        // 上面的狀態守衛 UPDATE 已保證只會轉一次，重送會 409 而到不了這裡
        let uid: Uuid = sqlx::query_scalar("SELECT user_id FROM point_wallets WHERE id = $1").bind(wallet).fetch_one(&mut *tx).await?;
        crate::notify::enqueue(&mut *tx, uid, "cash_refund.updated", json!({ "amount": amount, "status": to, "note": reason })).await?;
    }
    let out: Value = sqlx::query_scalar(
        "SELECT jsonb_build_object('id', id, 'amount', amount, 'status', status, 'reject_reason', reject_reason,
                'processed_by', processed_by, 'completed_at', completed_at) FROM cash_refund_requests WHERE id = $1")
        .bind(id).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    Ok(Json(out))
}

// ================= 欠款 =================

/// GET /admin/wallet-debts?status=（open | settled | written_off，預設 open）
async fn debt_list(
    State(st): State<AppState>,
    Staff(_): Staff,
    Query(q): Query<PageQ>,
) -> R<Response> {
    let status = q.status.clone().unwrap_or_else(|| "open".into());
    if !["open", "settled", "written_off"].contains(&status.as_str()) {
        return Err(AppError::invalid("/status", "ENUM", "status 不正確"));
    }
    let rows: Vec<(Uuid, Value)> = sqlx::query_as(
        "SELECT d.id, jsonb_build_object('id', d.id, 'amount', d.amount, 'recovered_points', d.recovered_points, 'status', d.status, 'topup_id', d.topup_id,
                'user', jsonb_build_object('id', u.id, 'display_name', u.display_name, 'email', u.email),
                'note', d.note, 'resolved_at', d.resolved_at, 'created_at', d.created_at)
           FROM wallet_debts d JOIN point_wallets w ON w.id = d.wallet_id JOIN users u ON u.id = w.user_id
          WHERE d.status::text = $1 AND ($2::uuid IS NULL OR d.id > $2) ORDER BY d.id LIMIT $3")
        .bind(&status).bind(q.cursor).bind(limit(&q) + 1).fetch_all(&st.pool).await?;
    Ok(page(rows, limit(&q)))
}

/// PATCH /admin/wallet-debts/{id}：open → settled | written_off（note 必填）；已結案 → 409 INVALID_STATE_TRANSITION
async fn debt_patch(
    State(st): State<AppState>,
    Staff(sid): Staff,
    Path(id): Path<Uuid>,
    Json(b): Json<Value>,
) -> R<Json<Value>> {
    let to = b
        .get("status")
        .and_then(Value::as_str)
        .filter(|s| ["settled", "written_off"].contains(s))
        .ok_or_else(|| {
            AppError::invalid("/status", "ENUM", "status 必須是 settled | written_off")
        })?;
    let note = text_field(&b, "note", true)?;
    let mut tx = st.pool.begin().await?;
    let out: Option<Value> = sqlx::query_scalar(
        "UPDATE wallet_debts SET status = $2::wallet_debt_status, note = $3, resolved_by = $4, resolved_at = now()
          WHERE id = $1 AND status = 'open'
          RETURNING jsonb_build_object('id', id, 'status', status, 'note', note, 'resolved_at', resolved_at)")
        .bind(id).bind(to).bind(&note).bind(sid).fetch_optional(&mut *tx).await?;
    let Some(out) = out else {
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM wallet_debts WHERE id = $1)")
                .bind(id)
                .fetch_one(&mut *tx)
                .await?;
        return Err(if exists {
            bad_transition()
        } else {
            AppError::NotFound
        });
    };
    audit(
        &mut *tx,
        sid,
        "wallet_debt.resolve",
        "wallet_debts",
        Some(id),
        json!({ "status": to, "note": note }),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(out))
}

// ================= 拒付（4.3s，D41）=================

async fn system_audit(tx: &mut Tx<'_>, action: &str, topup: Uuid, diff: Value) -> R<()> {
    sqlx::query("INSERT INTO audit_logs (actor_type, action, entity, entity_id, diff) VALUES ('system', $1, 'topups', $2, $3)")
        .bind(action).bind(topup).bind(diff).execute(&mut **tx).await?;
    Ok(())
}

/// 鎖 topups 列（先）與其錢包（後），回傳 (wallet_id, amount, status)。找不到 → NotFound。
async fn lock_topup(tx: &mut Tx<'_>, topup_id: Uuid) -> R<(Uuid, i64, String)> {
    let t: (Uuid, i64, String) = sqlx::query_as(
        "SELECT wallet_id, amount, status::text FROM topups WHERE id = $1 FOR UPDATE",
    )
    .bind(topup_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(AppError::NotFound)?;
    points::lock_wallets(tx, &[t.0]).await?;
    Ok(t)
}

async fn notify_wallet(tx: &mut Tx<'_>, wallet: Uuid, kind: &str, payload: Value) -> R<()> {
    let uid: Uuid = sqlx::query_scalar("SELECT user_id FROM point_wallets WHERE id = $1").bind(wallet).fetch_one(&mut **tx).await?;
    crate::notify::enqueue(&mut **tx, uid, kind, payload).await?;
    Ok(())
}

/// 爭議開始：paid → disputed 並凍結錢包。已不是 paid（重送 / 已處理）→ false，什麼都不動。
pub async fn dispute_open(tx: &mut Tx<'_>, topup_id: Uuid) -> R<bool> {
    let (wallet, amount, status) = lock_topup(tx, topup_id).await?;
    if status != "paid" {
        return Ok(false);
    }
    sqlx::query("UPDATE topups SET status = 'disputed', updated_at = now() WHERE id = $1")
        .bind(topup_id)
        .execute(&mut **tx)
        .await?;
    sqlx::query("UPDATE point_wallets SET status = 'frozen' WHERE id = $1")
        .bind(wallet)
        .execute(&mut **tx)
        .await?;
    system_audit(tx, "topup.disputed", topup_id, json!({ "amount": amount })).await?;
    notify_wallet(tx, wallet, "wallet.frozen", json!({})).await?;
    Ok(true)
}

/// 爭議勝訴：disputed → paid，錢包恢復 active（若沒有其他爭議中的儲值）。不是 disputed → false。
pub async fn dispute_won(tx: &mut Tx<'_>, topup_id: Uuid) -> R<bool> {
    let (wallet, _, status) = lock_topup(tx, topup_id).await?;
    if status != "disputed" {
        return Ok(false);
    }
    sqlx::query("UPDATE topups SET status = 'paid', updated_at = now() WHERE id = $1")
        .bind(topup_id)
        .execute(&mut **tx)
        .await?;
    let unfrozen = sqlx::query("UPDATE point_wallets SET status = 'active' WHERE id = $1 AND status = 'frozen' AND NOT EXISTS (SELECT 1 FROM topups WHERE wallet_id = $1 AND status = 'disputed')")
        .bind(wallet)
        .execute(&mut **tx)
        .await?
        .rows_affected() > 0;
    if unfrozen {
        notify_wallet(tx, wallet, "wallet.unfrozen", json!({})).await?;
    }
    system_audit(tx, "topup.dispute_won", topup_id, json!({})).await?;
    Ok(true)
}

#[derive(Debug, PartialEq, Eq)]
pub enum ChargedBack {
    /// 已處理：扣回 `recovered` 點（該 topup 批次剩餘），其餘 `debt` 點記入 wallet_debts
    Applied { recovered: i64, debt: i64 },
    /// topup 不是 paid / disputed（重送 / 已處理）：冪等，什麼都不動
    Ignored,
}

/// 拒付敗訴：paid | disputed → charged_back；先扣回該 topup 批次剩餘點數（ledger adjustment，ref topup），
/// 已被花掉的部分記 wallet_debts（balance 仍 >= 0），錢包凍結。重送冪等。
/// 認捐中的點數視為已花掉（不從 pledged 認捐追回）；之後這些點數若退回（撤回認捐、現金退款被拒絕），
/// 欠款未結清時由 points::offset_charged_back 抵銷 wallet_debts.recovered_points，不復活為可退現金的點數。
pub async fn charge_back(tx: &mut Tx<'_>, topup_id: Uuid) -> R<ChargedBack> {
    let (wallet, amount, status) = lock_topup(tx, topup_id).await?;
    if status != "paid" && status != "disputed" {
        return Ok(ChargedBack::Ignored);
    }
    sqlx::query("UPDATE topups SET status = 'charged_back', updated_at = now() WHERE id = $1")
        .bind(topup_id)
        .execute(&mut **tx)
        .await?;
    let lot: Option<(Uuid, i64)> = sqlx::query_as(
        "SELECT id, remaining_points FROM point_lots WHERE source = 'topup' AND source_ref_id = $1",
    )
    .bind(topup_id)
    .fetch_optional(&mut **tx)
    .await?;
    let recovered = lot.map_or(0, |l| l.1);
    if let Some((lot_id, rem)) = lot.filter(|l| l.1 > 0) {
        sqlx::query("UPDATE point_lots SET remaining_points = 0 WHERE id = $1")
            .bind(lot_id)
            .execute(&mut **tx)
            .await?;
        let after: i64 = sqlx::query_scalar(
            "UPDATE point_wallets SET balance = balance - $2 WHERE id = $1 RETURNING balance",
        )
        .bind(wallet)
        .bind(rem)
        .fetch_one(&mut **tx)
        .await?;
        points::ledger(
            tx,
            wallet,
            lot_id,
            -rem,
            after,
            &points::Entry {
                kind: "adjustment",
                ref_type: "topup",
                ref_id: Some(topup_id),
                note: Some("信用卡拒付，扣回儲值批次"),
                actor: None,
            },
        )
        .await?;
    }
    sqlx::query("UPDATE point_wallets SET status = 'frozen' WHERE id = $1")
        .bind(wallet)
        .execute(&mut **tx)
        .await?;
    let debt = amount - recovered;
    if debt > 0 {
        sqlx::query("INSERT INTO wallet_debts (wallet_id, topup_id, amount) VALUES ($1, $2, $3) ON CONFLICT (topup_id) DO NOTHING")
            .bind(wallet).bind(topup_id).bind(debt).execute(&mut **tx).await?;
    }
    system_audit(
        tx,
        "topup.charged_back",
        topup_id,
        json!({ "amount": amount, "recovered": recovered, "debt": debt }),
    )
    .await?;
    notify_wallet(tx, wallet, "wallet.chargeback", json!({ "amount": recovered, "debt": debt })).await?;
    Ok(ChargedBack::Applied { recovered, debt })
}

#[derive(Deserialize)]
struct TopupQ {
    cursor: Option<Uuid>,
    limit: Option<i64>,
    status: Option<String>,
    method: Option<String>,
    q: Option<String>,
    user_id: Option<Uuid>,
}

/// GET /admin/topups?status=&method=&q=&user_id=&limit=&cursor=：營運找儲值單 / 看爭議中清單。最新在前（uuid v7），cursor 為上一頁最後一筆 id。
/// q 不分大小寫比對姓名 / email 子字串（用 position 而非 LIKE，避免 % _ 被當萬用字元）；空白視為不篩選。
async fn topup_list(
    State(st): State<AppState>,
    Staff(_): Staff,
    Query(q): Query<TopupQ>,
) -> R<Response> {
    if q.method.as_deref().is_some_and(|m| {
        ![
            "card",
            "line_pay",
            "apple_pay",
            "cvs_code",
            "cvs_barcode",
            "atm",
        ]
        .contains(&m)
    }) {
        return Err(AppError::invalid("/method", "ENUM", "method 不正確"));
    }
    let kw = q.q.as_deref().map(str::trim).filter(|s| !s.is_empty());
    if q.status.as_deref().is_some_and(|s| {
        ![
            "pending",
            "paid",
            "failed",
            "expired",
            "refunded",
            "disputed",
            "charged_back",
        ]
        .contains(&s)
    }) {
        return Err(AppError::invalid("/status", "ENUM", "status 不正確"));
    }
    let lim = q.limit.unwrap_or(50).clamp(1, 100);
    let rows: Vec<(Uuid, Value)> = sqlx::query_as(
        "SELECT t.id, jsonb_build_object('id', t.id, 'amount', t.amount, 'method', t.method, 'status', t.status,
                'provider', t.provider, 'provider_txn_id', t.provider_txn_id, 'created_at', t.created_at, 'paid_at', t.paid_at,
                'user', jsonb_build_object('id', u.id, 'display_name', u.display_name, 'email', u.email),
                'wallet', jsonb_build_object('id', w.id, 'status', w.status))
           FROM topups t JOIN point_wallets w ON w.id = t.wallet_id JOIN users u ON u.id = w.user_id
          WHERE ($1::text IS NULL OR t.status::text = $1) AND ($2::uuid IS NULL OR t.id < $2)
            AND ($4::text IS NULL OR t.method::text = $4) AND ($5::uuid IS NULL OR u.id = $5)
            AND ($6::text IS NULL OR position(lower($6) IN lower(u.display_name)) > 0
                 OR COALESCE(position(lower($6) IN lower(u.email)) > 0, false))
          ORDER BY t.id DESC LIMIT $3")
        .bind(&q.status).bind(q.cursor).bind(lim + 1).bind(&q.method).bind(q.user_id).bind(kw).fetch_all(&st.pool).await?;
    Ok(page(rows, lim))
}

/// POST /admin/topups/{id}/dispute（D57）：{action: open | won | lost, note?}。Idempotency-Key 必填。
/// 已在目標狀態的重送（不同 key）回 200 applied=false 且不重複寫 audit；狀態不符 → 409 INVALID_STATE_TRANSITION。
/// won 對 paid 視為無動作（applied=false）：勝訴後狀態本來就是 paid。
async fn topup_dispute(
    State(st): State<AppState>,
    Staff(sid): Staff,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    body: Bytes,
) -> R<Response> {
    let key = idempotency::key(&headers)?;
    let b = parse(&body)?;
    let action = b
        .get("action")
        .and_then(Value::as_str)
        .filter(|a| ["open", "won", "lost"].contains(a))
        .ok_or_else(|| AppError::invalid("/action", "ENUM", "action 必須是 open | won | lost"))?;
    let note = match b.get("note") {
        None | Some(Value::Null) => None,
        Some(v) => Some(
            v.as_str()
                .map(|s| crate::validate::text(s, "/note", true))
                .transpose()?
                .filter(|s| s.chars().count() <= 500)
                .ok_or_else(|| AppError::invalid("/note", "RANGE", "note 需為 500 字以內"))?,
        ),
    };
    let method_path = format!("POST /admin/topups/{id}/dispute");
    let scope = format!("staff:{sid}:{method_path}");
    let hash = idempotency::request_hash(&method_path, &body);
    let mut tx = st.pool.begin().await?;
    if let idempotency::Begin::Replay(s, b) =
        idempotency::begin(&mut tx, &scope, key, &hash).await?
    {
        return Ok(json_resp(s as u16, &b, true));
    }
    // 三個函式都先鎖 topups 列再鎖錢包；鎖住後下面讀到的 status 不會再被別人改
    let (applied, outcome) = match action {
        "open" => (dispute_open(&mut tx, id).await?, None),
        "won" => (dispute_won(&mut tx, id).await?, None),
        _ => match charge_back(&mut tx, id).await? {
            ChargedBack::Applied { recovered, debt } => (true, Some((recovered, debt))),
            ChargedBack::Ignored => (false, None),
        },
    };
    let status: String = sqlx::query_scalar("SELECT status::text FROM topups WHERE id = $1")
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
    let already = match action {
        "open" => status == "disputed",
        "won" => status == "paid",
        _ => status == "charged_back",
    };
    if !applied && !already {
        return Err(bad_transition());
    }
    let mut resp = json!({ "topup_id": id, "status": status, "applied": applied });
    if let Some((recovered, debt)) = outcome {
        resp["recovered"] = json!(recovered);
        resp["debt"] = json!(debt);
    }
    if applied {
        let mut diff = resp.clone();
        diff["note"] = json!(note);
        audit(
            &mut *tx,
            sid,
            &format!("topup.dispute_{action}"),
            "topups",
            Some(id),
            diff,
        )
        .await?;
    }
    idempotency::finish(&mut tx, &scope, key, 200, &resp).await?;
    tx.commit().await?;
    Ok(json_resp(200, &resp, false))
}

// ================= 自動儲值設定（D24；觸發與扣款尚未實作）=================

const MONTH_START: &str =
    "date_trunc('month', now() AT TIME ZONE 'Asia/Taipei') AT TIME ZONE 'Asia/Taipei'";

async fn auto_json(pool: &sqlx::PgPool, user: Uuid) -> R<Value> {
    // 未設定過 = 預設值、enabled=false
    let (enabled, threshold, amount, cap, last4): (bool, i64, i64, i64, Option<String>) =
        sqlx::query_as("SELECT enabled, threshold_points, amount, monthly_cap_amount, card_last4::text FROM wallet_auto_topups WHERE user_id = $1")
            .bind(user).fetch_optional(pool).await?.unwrap_or((false, 500, 1000, 10_000, None));
    let used: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT COALESCE(sum(t.amount), 0)::bigint FROM topups t JOIN point_wallets w ON w.id = t.wallet_id
          WHERE w.user_id = $1 AND t.is_auto AND t.status IN ('pending', 'paid') AND t.created_at >= {MONTH_START}")))
        .bind(user).fetch_one(pool).await?;
    Ok(json!({
        "enabled": enabled, "threshold_points": threshold, "amount": amount, "monthly_cap_amount": cap,
        "method": "card", "card_last4": last4, "used_this_month": used, "limit_reached": used + amount > cap,
    }))
}

async fn auto_get(State(st): State<AppState>, user: CurrentUser) -> R<Response> {
    Ok(json_resp(200, &auto_json(&st.pool, user.id).await?, false))
}

/// PUT：建立 / 覆蓋設定（未帶的欄位沿用既有值或預設）。啟用需已綁卡；金流商尚未接入，帶 prime 一律 503 TOPUP_DISABLED。
async fn auto_put(State(st): State<AppState>, user: CurrentUser, body: Bytes) -> R<Response> {
    let b = parse(&body)?;
    if b.get("prime").is_some_and(|p| !p.is_null()) {
        return Err(AppError::problem(
            503,
            "TOPUP_DISABLED",
            "金流商尚未接入，暫時無法綁定信用卡",
        ));
    }
    let num = |k: &str, lo: i64, hi: i64| -> R<Option<i64>> {
        match b.get(k) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => v
                .as_i64()
                .filter(|n| (lo..=hi).contains(n))
                .map(Some)
                .ok_or_else(|| {
                    AppError::invalid(
                        &format!("/{k}"),
                        "RANGE",
                        &format!("{k} 必須介於 {lo} 與 {hi}"),
                    )
                }),
        }
    };
    let (threshold, amount, cap) = (
        num("threshold_points", 0, 50_000)?,
        num("amount", 100, 50_000)?,
        num("monthly_cap_amount", 1, 100_000)?,
    );
    let enabled = match b.get("enabled") {
        None | Some(Value::Null) => None,
        Some(v) => Some(
            v.as_bool()
                .ok_or_else(|| AppError::invalid("/enabled", "TYPE", "enabled 必須是布林"))?,
        ),
    };
    let mut tx = st.pool.begin().await?;
    let cur: Option<(bool, i64, i64, i64, bool)> = sqlx::query_as("SELECT enabled, threshold_points, amount, monthly_cap_amount, card_token_ref IS NOT NULL FROM wallet_auto_topups WHERE user_id = $1 FOR UPDATE")
        .bind(user.id).fetch_optional(&mut *tx).await?;
    let (c_en, c_th, c_am, c_cap, has_card) = cur.unwrap_or((false, 500, 1000, 10_000, false));
    let (enabled, threshold, amount, cap) = (
        enabled.unwrap_or(c_en),
        threshold.unwrap_or(c_th),
        amount.unwrap_or(c_am),
        cap.unwrap_or(c_cap),
    );
    if amount > cap {
        return Err(AppError::problem(
            409,
            "AUTO_TOPUP_LIMIT_REACHED",
            "單次儲值金額不可大於每月上限",
        ));
    }
    if enabled && !has_card {
        return Err(AppError::invalid(
            "/enabled",
            "CARD_REQUIRED",
            "尚未綁定信用卡，無法啟用自動儲值",
        ));
    }
    // 不用 ON CONFLICT：INSERT 的提議列會先過 CHECK（enabled 需有 card_token_ref），已有卡的列會被誤擋
    let updated = sqlx::query("UPDATE wallet_auto_topups SET enabled = $2, threshold_points = $3, amount = $4, monthly_cap_amount = $5 WHERE user_id = $1")
        .bind(user.id).bind(enabled).bind(threshold).bind(amount).bind(cap).execute(&mut *tx).await?;
    if updated.rows_affected() == 0 {
        sqlx::query("INSERT INTO wallet_auto_topups (user_id, enabled, threshold_points, amount, monthly_cap_amount) VALUES ($1, $2, $3, $4, $5)")
            .bind(user.id).bind(enabled).bind(threshold).bind(amount).bind(cap).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(json_resp(200, &auto_json(&st.pool, user.id).await?, false))
}

/// DELETE：關閉並清除 card token 參照（整列刪除，回到預設值）
async fn auto_delete(State(st): State<AppState>, user: CurrentUser) -> R<Response> {
    sqlx::query("DELETE FROM wallet_auto_topups WHERE user_id = $1")
        .bind(user.id)
        .execute(&st.pool)
        .await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}
