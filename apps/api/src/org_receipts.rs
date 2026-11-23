//! 據點實收登記：任何未停用成員（volunteer 起）登記品項實際收到的數量。
//! 未提供認領編號的實收只登記總量；有編號時核對品項與數量，私下通知該捐助者。
//! 鎖序：wishlist_items（FOR KEY SHARE）→ claims（FOR SHARE，僅有認領編號時）。
use crate::{
    error::AppError,
    idempotency,
    orgs::{audit_user, member_role, require_role},
    session::CurrentUser,
    validate, AppState,
};
use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::Response,
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

const MAX_QTY: i32 = 100_000;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/orgs/{id}/receipts", post(record))
        .route("/orgs/{id}/receipts/{receipt_id}/void", post(void))
        .route("/orgs/{id}/sites/{wishlist_id}/receipts", get(list))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Req {
    item_id: Uuid,
    claim_id: Option<Uuid>,
    qty: i32,
    note: Option<String>,
}

fn resp(status: u16, body: &Value, replay: bool) -> Response {
    let mut b = Response::builder()
        .status(StatusCode::from_u16(status).unwrap())
        .header(header::CACHE_CONTROL, "private, no-store")
        .header(header::CONTENT_TYPE, "application/json");
    if replay {
        b = b.header("Idempotency-Replayed", HeaderValue::from_static("true"));
    }
    b.body(axum::body::Body::from(body.to_string())).unwrap()
}

async fn record(
    State(st): State<AppState>,
    me: CurrentUser,
    Path(org): Path<Uuid>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, AppError> {
    require_role(&st.pool, org, me.id, "volunteer").await?;
    let key = idempotency::key(&headers)?;
    let req: Req = crate::error::json_body(&body, || {
        AppError::invalid("/", "INVALID", "請求內容格式錯誤")
    })?;
    if !(1..=MAX_QTY).contains(&req.qty) {
        return Err(AppError::invalid(
            "/qty",
            "RANGE",
            "qty 必須介於 1 與 100000",
        ));
    }
    let note = req.note.as_deref().map(str::trim).filter(|n| !n.is_empty());
    if note.is_some_and(|n| n.chars().count() > 200) {
        return Err(AppError::invalid("/note", "RANGE", "備註至多 200 字"));
    }

    let scope = format!("user:{}:POST /orgs/{org}/receipts", me.id);
    let hash = idempotency::request_hash(&format!("POST /orgs/{org}/receipts"), &body);
    let mut tx = st.pool.begin().await?;
    if let idempotency::Begin::Replay(s, b) =
        idempotency::begin(&mut tx, &scope, key, &hash).await?
    {
        return Ok(resp(s as u16, &b, true));
    }
    // 順便讀組織狀態（不鎖組織：撤銷與登記競態時以讀到的為準）。
    // 品項先取 FOR KEY SHARE（INSERT 的外鍵檢查本來就會取）：之後才鎖 claims，與認捐鎖序 wishlist_items → claims 一致，否則會與改認領的請求死結。
    let site: (Uuid, String, String) = sqlx::query_as(
        "SELECT w.id, w.site_status, o.verification_status::text FROM wishlist_items i JOIN wishlists w ON w.id = i.wishlist_id
         JOIN organizations o ON o.id = w.org_id
         WHERE i.id = $1 AND i.deleted_at IS NULL AND w.type = 'relief' AND w.org_id = $2 AND w.deleted_at IS NULL
           AND i.funding_mode <> 'support' FOR KEY SHARE OF i")
        .bind(req.item_id).bind(org).fetch_optional(&mut *tx).await?.ok_or(AppError::NotFound)?;
    if site.2 != "verified" {
        return Err(AppError::problem(
            409,
            "ORG_NOT_VERIFIED",
            "組織目前不是已驗證狀態，無法登記實收",
        ));
    }
    if site.1 == "closed" {
        return Err(AppError::problem(
            409,
            "SITE_CLOSED",
            "此據點已結束，無法再登記實收",
        ));
    }
    let donor: Option<Uuid> = if let Some(claim) = req.claim_id {
        sqlx::query_scalar(
            "SELECT c.user_id FROM claims c JOIN wishlist_items i ON i.id=c.item_id
              JOIN wishlists w ON w.id=i.wishlist_id
             WHERE c.id=$1 AND c.item_id=$2 AND c.qty=$3 AND i.funding_mode='quantity'
               AND NOT surprise_locked(w.surprise_mode,w.event_date)
               AND c.status IN ('reserved','purchased','delivered') FOR SHARE OF c",
        )
        .bind(claim)
        .bind(req.item_id)
        .bind(req.qty)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| {
            AppError::invalid("/claim_id", "CLAIM_MISMATCH", "認領編號、品項或數量不符")
        })?
    } else {
        None
    };
    let (id, created_at): (Uuid, chrono::DateTime<chrono::Utc>) = sqlx::query_as(
        "INSERT INTO receipts (item_id, wishlist_id, qty, note, recorded_by, claim_id) VALUES ($1,$2,$3,$4,$5,$6)
         ON CONFLICT (claim_id) WHERE claim_id IS NOT NULL AND voided_at IS NULL DO NOTHING RETURNING id, created_at")
        .bind(req.item_id).bind(site.0).bind(req.qty).bind(note).bind(me.id).bind(req.claim_id).fetch_optional(&mut *tx).await?
        .ok_or_else(|| AppError::problem(409, "CLAIM_ALREADY_RECEIVED", "此認領已登記實收"))?;
    if let Some(uid) = donor {
        crate::notify::enqueue(
            &mut *tx,
            uid,
            "claim.receipt_received",
            json!({"receipt_id":id,"claim_id":req.claim_id,"item_id":req.item_id,"wishlist_id":site.0}),
        )
        .await?;
    }
    audit_user(
        &mut *tx,
        me.id,
        "receipt.create",
        "receipt",
        Some(id),
        json!({"org_id": org, "wishlist_id": site.0, "item_id": req.item_id, "qty": req.qty}),
    )
    .await?;
    let out = json!({"id": id, "item_id": req.item_id, "qty": req.qty, "note": note, "created_at": created_at});
    idempotency::finish(&mut tx, &scope, key, 201, &out).await?;
    tx.commit().await?;
    Ok(resp(201, &out, false))
}

/// 據點的實收總覽：據點摘要 + 全部未刪除品項（沒有實收的 total = 0、entries = []），供志工登記頁使用。
async fn list(
    State(st): State<AppState>,
    me: CurrentUser,
    Path((org, wid)): Path<(Uuid, Uuid)>,
) -> Result<Json<Value>, AppError> {
    let role = require_role(&st.pool, org, me.id, "volunteer").await?;
    let coord = role != "volunteer";
    let site: Value = sqlx::query_scalar(
        "SELECT jsonb_build_object('id', w.id, 'title', w.title, 'slug', w.slug::text, 'site_status', w.site_status, 'site_address', w.site_address, 'org_status', o.verification_status::text)
           FROM wishlists w JOIN organizations o ON o.id = w.org_id
          WHERE w.id=$1 AND w.org_id=$2 AND w.type='relief' AND w.deleted_at IS NULL")
        .bind(wid).bind(org).fetch_optional(&st.pool).await?.ok_or(AppError::NotFound)?;
    let items: Vec<Value> = sqlx::query_scalar(
        "SELECT jsonb_build_object('item_id', i.id, 'title', i.title, 'funding_mode', i.funding_mode, 'qty_needed', i.qty_needed, 'category', i.category, 'urgency', i.urgency,
                'total', COALESCE(r.total, 0), 'entries', COALESCE(r.entries, '[]'::jsonb))
           FROM wishlist_items i
           LEFT JOIN LATERAL (SELECT COALESCE(sum(x.qty) FILTER (WHERE x.voided_at IS NULL), 0)::bigint AS total, jsonb_agg(jsonb_build_object(
                'id', x.id, 'qty', x.qty, 'note', x.note, 'recorded_by_name', u.display_name, 'created_at', x.created_at,
                'voided_at', x.voided_at, 'void_reason', x.void_reason,
                'can_void', x.voided_at IS NULL AND ($2 OR x.recorded_by = $3))
                ORDER BY x.created_at DESC, x.id DESC) AS entries
              FROM receipts x JOIN users u ON u.id = x.recorded_by WHERE x.item_id = i.id) r ON true
          WHERE i.wishlist_id = $1 AND i.deleted_at IS NULL AND i.funding_mode <> 'support' ORDER BY i.sort_order, i.id")
        .bind(wid).bind(coord).bind(me.id).fetch_all(&st.pool).await?;
    Ok(Json(json!({ "site": site, "items": items })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VoidReq {
    reason: String,
}

/// 作廢實收（更正帳目）：登記者本人或 coordinator 以上；組織非 verified、據點已結束時仍可作廢。
async fn void(
    State(st): State<AppState>,
    me: CurrentUser,
    Path((org, rid)): Path<(Uuid, Uuid)>,
    Json(req): Json<VoidReq>,
) -> Result<Json<Value>, AppError> {
    let role = member_role(&st.pool, org, me.id)
        .await?
        .ok_or(AppError::NotFound)?;
    let reason = validate::text(&req.reason, "/reason", false)?;
    if reason.is_empty() || reason.chars().count() > 200 {
        return Err(AppError::invalid(
            "/reason",
            "RANGE",
            "原因必填，至多 200 字",
        ));
    }
    let mut tx = st.pool.begin().await?;
    let r: (Uuid, Uuid, i32, Uuid) = sqlx::query_as(
        "SELECT r.wishlist_id, r.item_id, r.qty, r.recorded_by FROM receipts r JOIN wishlists w ON w.id = r.wishlist_id
          WHERE r.id = $1 AND w.org_id = $2")
        .bind(rid).bind(org).fetch_optional(&mut *tx).await?.ok_or(AppError::NotFound)?;
    if role == "volunteer" && r.3 != me.id {
        return Err(AppError::problem(
            403,
            "ORG_ROLE_REQUIRED",
            "此操作需要更高的組織角色",
        ));
    }
    // 條件式 UPDATE 一次完成，並行重複作廢只有一個成功
    let done: Option<(chrono::DateTime<chrono::Utc>,)> = sqlx::query_as(
        "UPDATE receipts SET voided_at = now(), voided_by = $2, void_reason = $3
          WHERE id = $1 AND voided_at IS NULL RETURNING voided_at",
    )
    .bind(rid)
    .bind(me.id)
    .bind(&reason)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((voided_at,)) = done else {
        return Err(AppError::problem(409, "ALREADY_VOIDED", "此實收紀錄已作廢"));
    };
    audit_user(
        &mut *tx,
        me.id,
        "receipt.void",
        "receipt",
        Some(rid),
        json!({"org_id": org, "wishlist_id": r.0, "item_id": r.1, "qty": r.2, "reason": reason}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(
        json!({"id": rid, "voided_at": voided_at, "void_reason": reason}),
    ))
}
