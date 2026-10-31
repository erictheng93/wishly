//! 平台審核：組織、許可（含撤銷）。供 staff 使用，審核結果寄 `org.review` 通知給 owner。
use crate::{
    admin::{audit, limit, page, PageQ, Staff},
    error::AppError,
    AppState,
};
use axum::{
    extract::{Path, Query, State},
    response::Response,
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/admin/orgs", get(list))
        .route("/admin/orgs/{id}", get(detail))
        .route("/admin/orgs/{id}/review", post(review))
        .route("/admin/permits/{id}/review", post(review_permit))
}

/// needs_changes 可標記的欄位：組織可編輯欄位 + 文件 / 許可。
const FLAGGABLE: &[&str] = &[
    "legal_name",
    "display_name",
    "org_type",
    "tax_id",
    "registration_no",
    "authority",
    "website",
    "intro",
    "logo_key",
    "rep_name",
    "contact_name",
    "contact_title",
    "contact_phone",
    "public_email",
    "public_phone",
    "documents",
    "permits",
];

async fn list(
    _: Staff,
    State(st): State<AppState>,
    Query(q): Query<PageQ>,
) -> Result<Response, AppError> {
    let lim = limit(&q);
    // 先送的先審；cursor 為上一頁最後一筆 id，以 (submitted_at, id) 接續
    // status=pending_permits 為偽狀態：已通過、且有待審許可的組織；每列帶 pending_permits 數量
    let rows: Vec<(Uuid, Value)> = sqlx::query_as(
        "SELECT o.id, to_jsonb(o) - 'contact_phone_enc'
                || jsonb_build_object('pending_permits', (SELECT count(*) FROM org_permits p WHERE p.org_id = o.id AND p.status = 'pending'))
           FROM organizations o
          WHERE o.verification_status <> 'draft'
            AND ($1::text IS NULL OR CASE WHEN $1 = 'pending_permits'
                   THEN o.verification_status = 'verified' AND EXISTS (SELECT 1 FROM org_permits p WHERE p.org_id = o.id AND p.status = 'pending')
                   ELSE o.verification_status = $1 END)
            AND ($2::uuid IS NULL OR (o.submitted_at, o.id) > (SELECT submitted_at, id FROM organizations WHERE id = $2))
          ORDER BY o.submitted_at ASC, o.id LIMIT $3")
        .bind(&q.status).bind(q.cursor).bind(lim + 1).fetch_all(&st.pool).await?;
    Ok(page(rows, lim))
}

async fn detail(
    _: Staff,
    State(st): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    let row: Option<(Value, Option<Vec<u8>>)> =
        sqlx::query_as("SELECT to_jsonb(o) - 'contact_phone_enc', contact_phone_enc FROM organizations o WHERE id = $1")
            .bind(id).fetch_optional(&st.pool).await?;
    let (mut org, enc) = row.ok_or_else(|| AppError::problem(404, "NOT_FOUND", "組織不存在"))?;
    org["contact_phone"] = json!(enc.as_deref().and_then(crate::sealed::open_str));
    let documents: Value = sqlx::query_scalar("SELECT COALESCE(jsonb_agg(to_jsonb(d) ORDER BY d.created_at), '[]') FROM org_documents d WHERE org_id = $1")
        .bind(id).fetch_one(&st.pool).await?;
    let permits: Value = sqlx::query_scalar("SELECT COALESCE(jsonb_agg(to_jsonb(p) ORDER BY p.created_at), '[]') FROM org_permits p WHERE org_id = $1")
        .bind(id).fetch_one(&st.pool).await?;
    let members: Value = sqlx::query_scalar(
        "SELECT COALESCE(jsonb_agg(jsonb_build_object('user_id', m.user_id, 'role', m.role, 'handle', m.handle,
                'disabled_at', m.disabled_at, 'display_name', u.display_name) ORDER BY m.created_at), '[]')
           FROM org_members m JOIN users u ON u.id = m.user_id WHERE m.org_id = $1")
        .bind(id).fetch_one(&st.pool).await?;
    Ok(Json(
        json!({ "org": org, "documents": documents, "permits": permits, "members": members }),
    ))
}

#[derive(Deserialize)]
struct Review {
    decision: String,
    note: Option<String>,
    flagged_fields: Option<Vec<String>>,
}

async fn review(
    Staff(sid): Staff,
    State(st): State<AppState>,
    Path(id): Path<Uuid>,
    Json(b): Json<Review>,
) -> Result<Json<Value>, AppError> {
    let flagged = b.flagged_fields.unwrap_or_default();
    if b.decision == "needs_changes"
        && (flagged.is_empty() || flagged.iter().any(|f| !FLAGGABLE.contains(&f.as_str())))
    {
        return Err(AppError::invalid(
            "/flagged_fields",
            "INVALID",
            "需至少一個合法欄位",
        ));
    }
    if !matches!(b.decision.as_str(), "approve" | "needs_changes" | "reject") {
        return Err(AppError::invalid("/decision", "INVALID", "decision 不合法"));
    }
    let mut tx = crate::begin_contended(&st.pool).await?;
    // 鎖組織列：兩位 staff 同時審同一筆時，只有一個轉移會成功
    let (status, display_name): (String, String) = sqlx::query_as(
        "SELECT verification_status, display_name FROM organizations WHERE id = $1 FOR UPDATE",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AppError::problem(404, "NOT_FOUND", "組織不存在"))?;
    let allowed = match b.decision.as_str() {
        "reject" => matches!(status.as_str(), "pending" | "verified"),
        _ => status == "pending",
    };
    if !allowed {
        return Err(AppError::problem(
            409,
            "INVALID_STATE",
            format!("目前狀態 {status} 不可執行 {}", b.decision),
        ));
    }
    let new_status = match b.decision.as_str() {
        "approve" => "verified",
        "needs_changes" => "needs_changes",
        _ => "rejected",
    };
    if new_status == "verified" {
        let ok: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM org_permits WHERE org_id = $1 AND status = 'approved')",
        )
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
        if !ok {
            return Err(AppError::problem(
                409,
                "PERMIT_REQUIRED",
                "需至少一張已核准的勸募許可",
            ));
        }
    }
    sqlx::query("UPDATE organizations SET verification_status = $2, review_note = $3, flagged_fields = $4,
                   verified_at = CASE WHEN $2 = 'verified' THEN now() END,
                   resubmit_after = CASE WHEN $2 = 'rejected' THEN now() + interval '30 days' END WHERE id = $1")
        .bind(id).bind(new_status).bind(&b.note)
        .bind(if new_status == "needs_changes" { flagged.clone() } else { vec![] })
        .execute(&mut *tx).await?;
    // 撤銷：已上線的救災據點降為草稿（觸發器只擋 status='active'，降級不受限）
    let slugs: Vec<String> = if status == "verified" {
        sqlx::query_scalar("UPDATE wishlists SET status = 'draft' WHERE type = 'relief' AND org_id = $1 AND status = 'active' RETURNING slug")
            .bind(id).fetch_all(&mut *tx).await?
    } else {
        vec![]
    };
    // 撤銷時才需要清事件頁與組織頁
    let (ev, org_slug): (Vec<String>, Option<String>) = if status == "verified" {
        (sqlx::query_scalar("SELECT DISTINCT e.slug::text FROM relief_events e JOIN wishlists w ON w.event_id = e.id WHERE w.type = 'relief' AND w.org_id = $1")
            .bind(id).fetch_all(&mut *tx).await?,
         sqlx::query_scalar("SELECT slug::text FROM organizations WHERE id = $1").bind(id).fetch_optional(&mut *tx).await?)
    } else {
        (vec![], None)
    };
    sqlx::query("INSERT INTO notifications (user_id, channel, kind, payload)
                 SELECT m.user_id, 'email'::notification_channel, 'org.review',
                        jsonb_build_object('org_id', $1::uuid, 'display_name', $2::text, 'decision', $3::text, 'note', $4::text, 'revoked', $5::bool)
                   FROM org_members m JOIN users u ON u.id = m.user_id
                  WHERE m.org_id = $1 AND m.role = 'owner' AND m.disabled_at IS NULL AND u.deleted_at IS NULL")
        .bind(id).bind(&display_name).bind(new_status).bind(&b.note).bind(status == "verified" && new_status == "rejected").execute(&mut *tx).await?;
    audit(&mut *tx, sid, "org.review", "organizations", Some(id),
          json!({ "decision": b.decision, "from": status, "to": new_status, "note": b.note, "flagged_fields": flagged })).await?;
    tx.commit().await?;
    if !slugs.is_empty() {
        crate::cdn::purge_wishlists(slugs.iter().map(|s| s.trim().to_string()).collect());
    }
    crate::cdn::purge_events(ev);
    if let Some(o) = org_slug {
        crate::cdn::purge_org(o);
    }
    Ok(Json(json!({ "id": id, "verification_status": new_status })))
}

#[derive(Deserialize)]
struct PermitReview {
    decision: String,
    note: Option<String>,
}

async fn review_permit(
    Staff(sid): Staff,
    State(st): State<AppState>,
    Path(id): Path<Uuid>,
    Json(b): Json<PermitReview>,
) -> Result<Json<Value>, AppError> {
    let new_status = match b.decision.as_str() {
        "approve" => "approved",
        "reject" => "rejected",
        _ => return Err(AppError::invalid("/decision", "INVALID", "decision 不合法")),
    };
    let mut tx = crate::begin_contended(&st.pool).await?;
    // 鎖序：組織 → 許可（與組織審核一致）
    let org_id: Uuid = sqlx::query_scalar("SELECT org_id FROM org_permits WHERE id = $1")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| AppError::problem(404, "NOT_FOUND", "許可不存在"))?;
    let (display_name, org_slug): (String, String) = sqlx::query_as(
        "SELECT display_name, slug::text FROM organizations WHERE id = $1 FOR UPDATE",
    )
    .bind(org_id)
    .fetch_one(&mut *tx)
    .await?;
    let (status, permit_no): (String, String) =
        sqlx::query_as("SELECT status, permit_no FROM org_permits WHERE id = $1 FOR UPDATE")
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
    // pending 可核准 / 退件；approved 只能撤銷（reject）
    if !(status == "pending" || (status == "approved" && new_status == "rejected")) {
        return Err(AppError::problem(
            409,
            "INVALID_STATE",
            format!("許可目前狀態 {status} 不可執行 {}", b.decision),
        ));
    }
    sqlx::query("UPDATE org_permits SET status = $2, review_note = $3 WHERE id = $1")
        .bind(id)
        .bind(new_status)
        .bind(&b.note)
        .execute(&mut *tx)
        .await?;
    // 撤銷已核准許可：使用此許可的上線據點暫停（不改組織狀態）
    let slugs: Vec<String> = if status == "approved" {
        let slugs = sqlx::query_scalar("UPDATE wishlists SET site_status = 'paused' WHERE permit_id = $1 AND type = 'relief' AND site_status = 'active' RETURNING slug")
            .bind(id).fetch_all(&mut *tx).await?;
        sqlx::query("INSERT INTO notifications (user_id, channel, kind, payload)
                     SELECT m.user_id, 'email'::notification_channel, 'org.permit_revoked',
                            jsonb_build_object('org_id', $1::uuid, 'display_name', $2::text, 'permit_no', $3::text, 'note', $4::text)
                       FROM org_members m JOIN users u ON u.id = m.user_id
                      WHERE m.org_id = $1 AND m.role = 'owner' AND m.disabled_at IS NULL AND u.deleted_at IS NULL")
            .bind(org_id).bind(&display_name).bind(&permit_no).bind(&b.note).execute(&mut *tx).await?;
        slugs
    } else {
        // 送審中的許可被退件：同樣通知 owner（帶備註），不必回狀態頁才知道
        if new_status == "rejected" {
            sqlx::query("INSERT INTO notifications (user_id, channel, kind, payload)
                         SELECT m.user_id, 'email'::notification_channel, 'org.permit_rejected',
                                jsonb_build_object('org_id', $1::uuid, 'display_name', $2::text, 'permit_no', $3::text, 'note', $4::text)
                           FROM org_members m JOIN users u ON u.id = m.user_id
                          WHERE m.org_id = $1 AND m.role = 'owner' AND m.disabled_at IS NULL AND u.deleted_at IS NULL")
                .bind(org_id).bind(&display_name).bind(&permit_no).bind(&b.note).execute(&mut *tx).await?;
        }
        vec![]
    };
    let ev: Vec<String> = if slugs.is_empty() {
        vec![]
    } else {
        sqlx::query_scalar("SELECT DISTINCT e.slug::text FROM relief_events e JOIN wishlists w ON w.event_id = e.id WHERE w.type = 'relief' AND w.permit_id = $1")
            .bind(id).fetch_all(&mut *tx).await?
    };
    audit(
        &mut *tx,
        sid,
        "permit.review",
        "org_permits",
        Some(id),
        json!({ "decision": b.decision, "from": status, "note": b.note }),
    )
    .await?;
    tx.commit().await?;
    if !slugs.is_empty() {
        crate::cdn::purge_wishlists(slugs.iter().map(|s| s.trim().to_string()).collect());
        crate::cdn::purge_events(ev);
        crate::cdn::purge_org(org_slug); // 組織頁只列上線據點，被暫停的要消失
    }
    Ok(Json(json!({ "id": id, "status": new_status })))
}
