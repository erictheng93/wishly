//! 帳號安全活動（GET /me/security-activity）：讓本人看到自己帳號近期的敏感事件，及早發現被盜用。
//! 只讀 audit_logs；不回傳原始 diff，只白名單 method / provider；營運人員動作只露出動作與時間，不含身分與理由。
use axum::{extract::{Query, State}, http::header, response::IntoResponse, routing::get, Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::{error::AppError, session::CurrentUser, AppState};

pub fn routes() -> Router<AppState> {
    Router::new().route("/me/security-activity", get(list))
}

#[derive(Deserialize)]
struct Q {
    limit: Option<i64>,
    /// 上一頁最後一筆的 id（uuid_v7，時間序）
    before: Option<Uuid>,
}

// 本人動作白名單（auth.% 另以 LIKE 納入）。
const OWN_ACTIONS: &[&str] = &["account.export", "account.absorb", "wishlist.shipping_address.update"];
// 營運動作白名單；user.update 只在 blocked 狀態改變時才算（對應 account.suspended / restored），其餘（改 staff 權限）不外露。
const STAFF_ACTIONS: &[&str] = &["user.mfa_reset", "user.recovery", "user.identity_unlink"];

async fn list(
    u: CurrentUser,
    State(st): State<AppState>,
    Query(q): Query<Q>,
) -> Result<impl IntoResponse, AppError> {
    let limit = q.limit.unwrap_or(20).clamp(1, 50);
    // TODO(audit-infra): 合併後加 user_agent
    let rows: Vec<Value> = sqlx::query_scalar(
        "SELECT jsonb_build_object('id', id, 'action', action, 'created_at', created_at, 'ip', ip_s,
                  'method', method, 'provider', provider, 'actor_type', actor_type)
         FROM (
           SELECT id, created_at, actor_type::text AS actor_type, action, host(ip) AS ip_s,
                  diff->>'method' AS method, diff->>'provider' AS provider
             FROM audit_logs
            WHERE actor_type = 'user' AND actor_id = $1
              AND (action LIKE 'auth.%' OR action = ANY($2))
           UNION ALL
           SELECT id, created_at, 'staff', 
                  CASE WHEN action = 'user.update'
                       THEN CASE WHEN diff#>>'{after,blocked}' = 'true' THEN 'account.suspended' ELSE 'account.restored' END
                       ELSE action END,
                  NULL, NULL, NULL
             FROM audit_logs
            WHERE actor_type = 'staff' AND entity = 'users' AND entity_id = $1
              AND (action = ANY($3)
                   OR (action = 'user.update' AND diff#>>'{before,blocked}' IS DISTINCT FROM diff#>>'{after,blocked}'))
         ) t
         WHERE ($4::uuid IS NULL OR id < $4)
         ORDER BY id DESC LIMIT $5",
    )
    .bind(u.id)
    .bind(OWN_ACTIONS)
    .bind(STAFF_ACTIONS)
    .bind(q.before)
    .bind(limit + 1)
    .fetch_all(&st.pool)
    .await?;
    let more = rows.len() as i64 > limit;
    let data: Vec<Value> = rows.into_iter().take(limit as usize).collect();
    let next = if more { data.last().map(|r| r["id"].clone()) } else { None };
    Ok(([(header::CACHE_CONTROL, "no-store")], Json(json!({ "data": data, "next_cursor": next }))))
}
