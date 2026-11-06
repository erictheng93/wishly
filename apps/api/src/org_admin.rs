//! 平台審核：組織、許可（含撤銷）。供 staff 使用，審核結果寄 `org.review` 通知給 owner。
use crate::{
    admin::{audit, limit, page, PageQ, Staff, MODERATOR},
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
        .route("/admin/orgs/{id}/restore", post(restore))
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
    _: Staff<MODERATOR>,
    State(st): State<AppState>,
    Query(q): Query<PageQ>,
) -> Result<Response, AppError> {
    let lim = limit(&q);
    // 先送的先審；cursor 為上一頁最後一筆 id，以 (submitted_at, id) 接續
    // status=pending_permits 為偽狀態：已通過、且有待審許可的組織；每列帶 pending_permits 數量
    let rows: Vec<(Uuid, Value)> = sqlx::query_as(
        "SELECT o.id, to_jsonb(o) - 'contact_phone_enc'
                || jsonb_build_object('pending_permits', (SELECT count(*) FROM org_permits p WHERE p.org_id = o.id AND p.status = 'pending'),
                    'restorable_until', CASE WHEN o.verification_status = 'rejected' AND o.revoked_at > now() - interval '7 days' THEN o.revoked_at + interval '7 days' END,
                    'revoke_paused_sites', (SELECT count(*) FROM wishlists w WHERE w.org_id = o.id AND w.revoke_paused_at IS NOT NULL))
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
    _: Staff<MODERATOR>,
    State(st): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    let row: Option<(Value, Option<Vec<u8>>)> =
        sqlx::query_as("SELECT to_jsonb(o) - 'contact_phone_enc', contact_phone_enc FROM organizations o WHERE id = $1")
            .bind(id).fetch_optional(&st.pool).await?;
    let (mut org, enc) = row.ok_or_else(|| AppError::problem(404, "NOT_FOUND", "組織不存在"))?;
    org["contact_phone"] = json!(enc.as_deref().and_then(crate::sealed::open_str));
    // 撤銷前讓 staff 看到影響：撤銷會把這些上線中的據點改回草稿
    let live: i64 = sqlx::query_scalar("SELECT count(*) FROM wishlists WHERE org_id = $1 AND type = 'relief' AND status = 'active' AND deleted_at IS NULL")
        .bind(id).fetch_one(&st.pool).await?;
    org["live_sites"] = json!(live);
    let (until, paused): (Option<chrono::DateTime<chrono::Utc>>, i64) = sqlx::query_as(
        "SELECT CASE WHEN o.verification_status = 'rejected' AND o.revoked_at > now() - interval '7 days' THEN o.revoked_at + interval '7 days' END,
                (SELECT count(*) FROM wishlists w WHERE w.org_id = o.id AND w.revoke_paused_at IS NOT NULL) FROM organizations o WHERE o.id = $1")
        .bind(id).fetch_one(&st.pool).await?;
    org["restorable_until"] = json!(until);
    org["revoke_paused_sites"] = json!(paused);
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

fn review_note(note: Option<&str>, required: bool) -> Result<Option<String>, AppError> {
    let note = crate::validate::optional_note(note, "/note", 500)?;
    if required && note.is_none() {
        return Err(AppError::invalid(
            "/note",
            "REQUIRED",
            "退件／撤銷必須填原因",
        ));
    }
    Ok(note)
}

async fn review(
    Staff(sid): Staff<MODERATOR>,
    State(st): State<AppState>,
    Path(id): Path<Uuid>,
    Json(mut b): Json<Review>,
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
    b.note = review_note(b.note.as_deref(), b.decision == "reject")?;
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
    sqlx::query(
        "UPDATE organizations SET verification_status = $2, review_note = $3, flagged_fields = $4,
                   verified_at = CASE WHEN $2 = 'verified' THEN now() END,
                   resubmit_after = CASE WHEN $2 = 'rejected' THEN now() + interval '30 days' END,
                   revoked_at = CASE WHEN $2 = 'rejected' AND $5 THEN now() END WHERE id = $1",
    )
    .bind(id)
    .bind(new_status)
    .bind(&b.note)
    .bind(if new_status == "needs_changes" {
        flagged.clone()
    } else {
        vec![]
    })
    .bind(status == "verified")
    .execute(&mut *tx)
    .await?;
    // 撤銷已驗證的組織：其待處理檢舉視為已處置。鎖序 organizations（上面 FOR UPDATE）→ content_reports；
    // status = 'open' 不可省，以免覆寫已駁回的檢舉
    if status == "verified" && new_status == "rejected" {
        let reporters: Vec<Option<Uuid>> = sqlx::query_scalar("UPDATE content_reports SET status = 'actioned', handled_by = $2, handled_at = now(), resolution_note = $3 WHERE reported_org_id = $1 AND status = 'open' RETURNING reporter_user_id")
            .bind(id).bind(sid).bind(&b.note).fetch_all(&mut *tx).await?;
        crate::notify::enqueue_report_resolved(&mut *tx, &reporters, "actioned").await?;
    }
    // 重新核准：舊撤銷留下的標記作廢，以免日後另一次撤銷的恢復動到無關的草稿據點
    if new_status == "verified" {
        sqlx::query("UPDATE wishlists SET revoke_paused_at = NULL WHERE org_id = $1 AND revoke_paused_at IS NOT NULL")
            .bind(id).execute(&mut *tx).await?;
    }
    // 撤銷：已上線的救災據點降為草稿（觸發器只擋 status='active'，降級不受限）；revoke_paused_at 標記供 7 天內恢復
    let slugs: Vec<String> = if status == "verified" {
        let down: Vec<(String, Uuid)> = sqlx::query_as("UPDATE wishlists SET status = 'draft', revoke_paused_at = now() WHERE type = 'relief' AND org_id = $1 AND status = 'active' RETURNING slug, id")
            .bind(id).fetch_all(&mut *tx).await?;
        crate::notify::enqueue_site_paused(&mut *tx, &down.iter().map(|p| p.1).collect::<Vec<_>>())
            .await?;
        down.into_iter().map(|p| p.0).collect()
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

#[derive(Deserialize)]
struct Restore {
    reason: Option<String>,
}

/// 撤銷後 7 天內恢復認證。鎖序 organizations（FOR UPDATE）→ relief_events（FOR SHARE，依 id）→ wishlists，與 review / 據點上線一致。
/// 事件在撤銷時只清快取、未被改動，故無需還原。撤銷時關閉的檢舉維持已處置；review_note 清空（原撤銷理由留在稽核紀錄）。
/// 據點只在組織已 verified、許可仍有效、事件仍可用時回到 active（SQL 先篩，trigger 不會中止交易）；其餘維持草稿並回報 skipped。
async fn restore(
    Staff(sid): Staff<MODERATOR>,
    State(st): State<AppState>,
    Path(id): Path<Uuid>,
    Json(b): Json<Restore>,
) -> Result<Json<Value>, AppError> {
    let reason = crate::validate::optional_note(b.reason.as_deref(), "/reason", 500)?
        .ok_or_else(|| AppError::invalid("/reason", "REQUIRED", "恢復認證必須填原因"))?;
    let mut tx = crate::begin_contended(&st.pool).await?;
    let (status, display_name, org_slug, window_ok, revoked): (String, String, String, bool, bool) = sqlx::query_as(
        "SELECT verification_status, display_name, slug::text, COALESCE(revoked_at > now() - interval '7 days', false), revoked_at IS NOT NULL
           FROM organizations WHERE id = $1 FOR UPDATE")
        .bind(id).fetch_optional(&mut *tx).await?
        .ok_or_else(|| AppError::problem(404, "NOT_FOUND", "組織不存在"))?;
    if status != "rejected" || !revoked {
        return Err(AppError::problem(
            409,
            "RESTORE_NOT_ALLOWED",
            "此組織不是被撤銷的狀態，不能恢復認證",
        ));
    }
    if !window_ok {
        return Err(AppError::problem(
            409,
            "RESTORE_NOT_ALLOWED",
            "撤銷已超過 7 天，不能恢復認證",
        ));
    }
    sqlx::query("UPDATE organizations SET verification_status = 'verified', verified_at = now(), resubmit_after = NULL, revoked_at = NULL, review_note = NULL, flagged_fields = '{}' WHERE id = $1")
        .bind(id).execute(&mut *tx).await?;
    sqlx::query("SELECT id FROM relief_events WHERE id IN (SELECT event_id FROM wishlists WHERE org_id = $1 AND revoke_paused_at IS NOT NULL) ORDER BY id FOR SHARE")
        .bind(id).fetch_all(&mut *tx).await?;
    // 候選據點與判定；必須仍是草稿（撤銷後創建者無法改動，保險起見仍檢查）
    let cands: Vec<(Uuid, String, Option<String>)> = sqlx::query_as(
        "SELECT w.id, w.slug::text,
                CASE WHEN w.deleted_at IS NOT NULL OR w.status <> 'draft' THEN 'not_draft'
                     WHEN NOT EXISTS (SELECT 1 FROM org_permits p WHERE p.id = w.permit_id AND p.org_id = w.org_id AND p.status = 'approved'
                                      AND (now() AT TIME ZONE 'Asia/Taipei')::date BETWEEN p.valid_from AND p.valid_to) THEN 'permit_invalid'
                     WHEN NOT EXISTS (SELECT 1 FROM relief_events e WHERE e.id = w.event_id AND e.merged_into IS NULL AND e.moderation_status::text <> 'hidden') THEN 'event_unavailable'
                END
           FROM wishlists w WHERE w.org_id = $1 AND w.type = 'relief' AND w.revoke_paused_at IS NOT NULL ORDER BY w.id FOR UPDATE OF w")
        .bind(id).fetch_all(&mut *tx).await?;
    let ok: Vec<Uuid> = cands
        .iter()
        .filter(|c| c.2.is_none())
        .map(|c| c.0)
        .collect();
    sqlx::query("UPDATE wishlists SET status = 'active' WHERE id = ANY($1) AND revoke_paused_at IS NOT NULL AND status = 'draft'")
        .bind(&ok).execute(&mut *tx).await?;
    sqlx::query("UPDATE wishlists SET revoke_paused_at = NULL WHERE org_id = $1 AND revoke_paused_at IS NOT NULL")
        .bind(id).execute(&mut *tx).await?;
    let restored: Vec<String> = cands
        .iter()
        .filter(|c| c.2.is_none())
        .map(|c| c.1.trim().to_string())
        .collect();
    let skipped: Vec<Value> = cands
        .iter()
        .filter_map(|c| {
            c.2.as_ref()
                .map(|r| json!({ "slug": c.1.trim(), "reason": r }))
        })
        .collect();
    // 沿用 org.review（decision=verified）通知 owner；備註帶恢復原因
    crate::notify::enqueue_org_owners(&mut *tx, id, "org.review",
        json!({"org_id": id, "display_name": display_name, "decision": "verified", "note": reason, "revoked": false})).await?;
    audit(
        &mut *tx,
        sid,
        "org.restore",
        "organizations",
        Some(id),
        json!({ "reason": reason, "restored": restored.len(), "skipped": skipped.len() }),
    )
    .await?;
    let ev: Vec<String> = sqlx::query_scalar("SELECT DISTINCT e.slug::text FROM relief_events e JOIN wishlists w ON w.event_id = e.id WHERE w.type = 'relief' AND w.id = ANY($1)")
        .bind(&ok).fetch_all(&mut *tx).await?;
    tx.commit().await?;
    if !restored.is_empty() {
        crate::cdn::purge_wishlists(restored.clone());
        crate::cdn::purge_events(ev);
    }
    crate::cdn::purge_org(org_slug);
    Ok(Json(
        json!({ "org": { "id": id, "verification_status": "verified" }, "restored": restored, "skipped": skipped }),
    ))
}

async fn review_permit(
    Staff(sid): Staff<MODERATOR>,
    State(st): State<AppState>,
    Path(id): Path<Uuid>,
    Json(mut b): Json<PermitReview>,
) -> Result<Json<Value>, AppError> {
    let new_status = match b.decision.as_str() {
        "approve" => "approved",
        "reject" => "rejected",
        _ => return Err(AppError::invalid("/decision", "INVALID", "decision 不合法")),
    };
    b.note = review_note(b.note.as_deref(), b.decision == "reject")?;
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
        let paused: Vec<(String, Uuid)> = sqlx::query_as("UPDATE wishlists SET site_status = 'paused' WHERE permit_id = $1 AND type = 'relief' AND site_status = 'active' RETURNING slug, id")
            .bind(id).fetch_all(&mut *tx).await?;
        // 已暫停的據點也會公開顯示許可：碰 updated_at 讓 ETag 失效，並一起 purge
        let others: Vec<String> = sqlx::query_scalar("UPDATE wishlists SET updated_at = now() WHERE permit_id = $1 AND type = 'relief' AND site_status <> 'active' RETURNING slug")
            .bind(id).fetch_all(&mut *tx).await?;
        let slugs: Vec<String> = paused.iter().map(|p| p.0.clone()).chain(others).collect();
        crate::notify::enqueue_site_paused(
            &mut *tx,
            &paused.iter().map(|p| p.1).collect::<Vec<_>>(),
        )
        .await?;
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
    // 退件走 org.permit_rejected；這裡只補核准
    if status == "pending" && new_status == "approved" {
        crate::notify::enqueue_org_owners(&mut *tx, org_id, "org.permit_reviewed",
            json!({"org_id": org_id, "display_name": display_name, "permit_no": permit_no, "note": b.note})).await?;
    }
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
