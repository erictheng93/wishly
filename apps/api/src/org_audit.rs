//! 組織稽核軌跡：owner / coordinator 唯讀自家組織相關的 audit_logs。
//! 範圍：entity_id 為組織本身、diff.org_id 指向本組織、或指向本組織的邀請 / 文件 / 許可 / 事件 /
//! 據點（wishlists.org_id）及其品項。claims / contributions 一律排除（D58：捐助者身分不外流）。
//! 不回 actor_id / ip / user_agent；僅本組織（含已停用）成員顯示名稱，營運只標 staff。
use crate::{admin::page, error::AppError, orgs::require_role, session::CurrentUser, AppState};
use axum::{
    extract::{Path, Query, State},
    response::Response,
    routing::get,
    Router,
};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

pub fn routes() -> Router<AppState> {
    Router::new().route("/orgs/{id}/audit-logs", get(list))
}

#[derive(Deserialize)]
struct Q {
    cursor: Option<Uuid>,
    limit: Option<i64>,
    action: Option<String>,
    entity: Option<String>,
    from: Option<chrono::NaiveDate>,
    to: Option<chrono::NaiveDate>,
}

async fn list(
    State(st): State<AppState>,
    u: CurrentUser,
    Path(org): Path<Uuid>,
    Query(q): Query<Q>,
) -> Result<Response, AppError> {
    require_role(&st.pool, org, u.id, "coordinator").await?;
    if q.from.zip(q.to).is_some_and(|(from, to)| from > to) {
        return Err(AppError::invalid(
            "/to",
            "RANGE",
            "結束日期不可早於開始日期",
        ));
    }
    let lim = q.limit.unwrap_or(20).clamp(1, 100);
    let rows: Vec<(Uuid, Value)> = sqlx::query_as(
        "SELECT a.id, jsonb_build_object('id',a.id,'action',a.action,'entity',a.entity,'entity_id',a.entity_id,
            'diff',a.diff,'created_at',a.created_at,
            'actor', CASE a.actor_type::text
              WHEN 'user' THEN jsonb_build_object('type','user','name',u.display_name)
              WHEN 'staff' THEN jsonb_build_object('type','staff','name',NULL)
              END)
         FROM audit_logs a
         LEFT JOIN org_members m ON m.org_id=$1 AND a.actor_type::text='user' AND m.user_id=a.actor_id
         LEFT JOIN users u ON u.id=m.user_id
         WHERE ($2::uuid IS NULL OR a.id<$2)
           AND a.entity NOT IN ('claims','contributions') AND a.action NOT LIKE 'admin.%'
           AND (a.entity_id=$1 OR a.diff->>'org_id'=$1::text
             OR a.entity_id IN (SELECT id FROM org_invites WHERE org_id=$1)
             OR a.entity_id IN (SELECT id FROM org_documents WHERE org_id=$1)
             OR a.entity_id IN (SELECT id FROM org_permits WHERE org_id=$1)
             OR a.entity_id IN (SELECT id FROM relief_events WHERE created_by_org=$1)
             OR a.entity_id IN (SELECT id FROM wishlists WHERE org_id=$1)
             OR a.entity_id IN (SELECT i.id FROM wishlist_items i JOIN wishlists w ON w.id=i.wishlist_id WHERE w.org_id=$1))
           AND ($3::text IS NULL OR a.action=$3) AND ($4::text IS NULL OR a.entity=$4)
           AND ($5::date IS NULL OR a.created_at >= ($5::date::timestamp AT TIME ZONE 'Asia/Taipei'))
           AND ($6::date IS NULL OR a.created_at < (($6::date+1)::timestamp AT TIME ZONE 'Asia/Taipei'))
         ORDER BY a.id DESC LIMIT $7",
    )
    .bind(org).bind(q.cursor).bind(&q.action).bind(&q.entity).bind(q.from).bind(q.to).bind(lim + 1)
    .fetch_all(&st.pool)
    .await?;
    Ok(page(rows, lim))
}
