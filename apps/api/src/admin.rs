//! 營運後台：STAFF 守衛 + 清單/使用者搜尋 + system-flags + 簡易統計。
//! 檢舉佇列與下架在 reports.rs（同樣使用 `Staff`）。
use crate::{error::AppError, session::CurrentUser, AppState};
use axum::{
    extract::{FromRequestParts, Path, Query, State},
    http::{header, request::Parts, HeaderValue},
    response::{IntoResponse, Response},
    routing::{delete, get, patch, post, put},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/admin/wishlists", get(list_wishlists))
        .route("/admin/users", get(list_users))
        .route("/admin/users/{id}", patch(update_user))
        .route("/admin/users/{id}/identities/{provider}", delete(unlink_identity))
        .route("/admin/users/{id}/recovery", post(recover_user))
        .route("/admin/audit-logs", get(list_audit_logs))
        .route("/admin/system-flags", get(list_flags))
        .route("/admin/system-flags/{key}", put(set_flag))
        .route("/admin/stats", get(stats))
}

/// 已登入且 is_staff；否則 401 / 403 STAFF_ONLY。
pub struct Staff(pub Uuid);

impl FromRequestParts<AppState> for Staff {
    type Rejection = AppError;
    async fn from_request_parts(parts: &mut Parts, st: &AppState) -> Result<Self, AppError> {
        let u = CurrentUser::from_request_parts(parts, st).await?;
        if u.is_staff { Ok(Staff(u.id)) } else { Err(AppError::problem(403, "STAFF_ONLY", "僅營運人員可使用")) }
    }
}

pub async fn audit<'e, E: sqlx::PgExecutor<'e>>(ex: E, staff: Uuid, action: &str, entity: &str, entity_id: Option<Uuid>, diff: Value) -> Result<(), AppError> {
    sqlx::query("INSERT INTO audit_logs (actor_type, actor_id, action, entity, entity_id, diff) VALUES ('staff', $1, $2, $3, $4, $5)")
        .bind(staff).bind(action).bind(entity).bind(entity_id).bind(diff).execute(ex).await?;
    Ok(())
}

#[derive(Deserialize)]
pub struct PageQ { pub cursor: Option<Uuid>, pub limit: Option<i64>, pub q: Option<String>, pub status: Option<String>, pub moderation_status: Option<String> }

pub fn limit(q: &PageQ) -> i64 { q.limit.unwrap_or(20).clamp(1, 100) }

/// rows 為 limit+1 筆 (id, json)；cursor 為最後一筆 id（uuid v7 時間序，opaque 給前端）。
pub fn page(mut rows: Vec<(Uuid, Value)>, limit: i64) -> Response {
    let next = if rows.len() as i64 > limit { rows.truncate(limit as usize); rows.last().map(|r| r.0) } else { None };
    let mut res = Json(json!({ "data": rows.into_iter().map(|r| r.1).collect::<Vec<_>>(), "next_cursor": next })).into_response();
    res.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("private, no-store"));
    res
}

async fn list_wishlists(State(st): State<AppState>, Staff(sid): Staff, Query(q): Query<PageQ>) -> Result<Response, AppError> {
    if let Some(m) = &q.moderation_status { if m != "ok" && m != "hidden" { return Err(AppError::invalid("/moderation_status", "INVALID", "ok | hidden")); } }
    enum_filter(q.status.as_deref(), &["draft", "active", "closed", "archived"], "/status")?;
    let qs = q.q.as_deref().map(str::trim).filter(|s| !s.is_empty());
    if let Some(s) = qs { audit(&st.pool, sid, "admin.search", "wishlists", None, json!({ "q": s })).await?; }
    let rows: Vec<(Uuid, Value)> = sqlx::query_as(
        "SELECT w.id, jsonb_build_object('id', w.id, 'slug', w.slug::text, 'title', w.title, 'status', w.status::text, 'type', w.type::text,
           'owner', jsonb_build_object('id', u.id, 'display_name', u.display_name, 'email', u.email),
           'moderation_status', w.moderation_status::text,
           'open_report_count', (SELECT count(*) FROM content_reports r WHERE r.wishlist_id = w.id AND r.status = 'open'),
           'created_at', w.created_at)
         FROM wishlists w JOIN users u ON u.id = w.owner_id
         WHERE w.deleted_at IS NULL
           AND ($1::text IS NULL OR w.slug::text = $1 OR left(u.email, length($1)) = lower($1))
           AND ($2::text IS NULL OR w.moderation_status::text = $2)
           AND ($3::uuid IS NULL OR w.id > $3)
           AND ($5::text IS NULL OR w.status::text=$5)
         ORDER BY w.id LIMIT $4")
        .bind(qs).bind(&q.moderation_status).bind(q.cursor).bind(limit(&q) + 1).bind(&q.status).fetch_all(&st.pool).await?;
    Ok(page(rows, limit(&q)))
}

#[derive(Deserialize)]
struct UserQ {
    cursor: Option<Uuid>, limit: Option<i64>, q: Option<String>, status: Option<String>, provider: Option<String>,
}

fn enum_filter(value: Option<&str>, allowed: &[&str], pointer: &str) -> Result<(), AppError> {
    if value.is_some_and(|v| !allowed.contains(&v)) { return Err(AppError::invalid(pointer, "ENUM", "篩選值不正確")); }
    Ok(())
}

async fn list_users(State(st): State<AppState>, Staff(sid): Staff, Query(q): Query<UserQ>) -> Result<Response, AppError> {
    let lim = q.limit.unwrap_or(20).clamp(1,100);
    enum_filter(q.provider.as_deref(), &["email", "line", "google"], "/provider")?;
    enum_filter(q.status.as_deref(), &["active", "blocked", "deleted", "staff"], "/status")?;
    let qs = q.q.as_deref().map(str::trim).filter(|s| !s.is_empty());
    if let Some(s) = qs { audit(&st.pool, sid, "admin.search", "users", None, json!({ "q": s })).await?; }
    let qid = qs.and_then(|s| s.parse::<Uuid>().ok());
    let rows: Vec<(Uuid, Value)> = sqlx::query_as(
        "SELECT u.id, jsonb_build_object('id', u.id, 'display_name', u.display_name, 'handle', u.handle, 'email', u.email, 'is_staff', u.is_staff,
           'blocked', u.blocked_at IS NOT NULL, 'blocked_at', u.blocked_at, 'deleted', u.deleted_at IS NOT NULL,
           'identities', (SELECT COALESCE(jsonb_agg(jsonb_build_object('provider', i.provider::text) ORDER BY i.provider::text), '[]') FROM auth_identities i WHERE i.user_id=u.id),
           'wishlist_count', (SELECT count(*) FROM wishlists w WHERE w.owner_id = u.id AND w.deleted_at IS NULL), 'created_at', u.created_at)
         FROM users u
         WHERE ($1::text IS NULL OR u.id=$2 OR strpos(lower(u.email),lower($1))>0 OR strpos(lower(u.display_name),lower($1))>0 OR strpos(lower(u.handle),lower($1))>0)
           AND ($3::uuid IS NULL OR u.id > $3)
           AND ($5::text IS NULL OR EXISTS(SELECT 1 FROM auth_identities i WHERE i.user_id=u.id AND i.provider::text=$5))
           AND ($6::text IS NULL OR ($6='active' AND u.deleted_at IS NULL AND u.blocked_at IS NULL)
                OR ($6='blocked' AND u.deleted_at IS NULL AND u.blocked_at IS NOT NULL)
                OR ($6='deleted' AND u.deleted_at IS NOT NULL)
                OR ($6='staff' AND u.is_staff AND u.deleted_at IS NULL AND u.blocked_at IS NULL))
         ORDER BY u.id LIMIT $4")
        .bind(qs).bind(qid).bind(q.cursor).bind(lim+1).bind(&q.provider).bind(&q.status).fetch_all(&st.pool).await?;
    Ok(page(rows, lim))
}

fn operation_reason(b: &Value) -> Result<String, AppError> {
    let reason = b.get("reason").and_then(Value::as_str).unwrap_or("");
    let reason = crate::validate::text(reason, "/reason", false)?;
    if !(1..=200).contains(&reason.chars().count()) { return Err(AppError::invalid("/reason", "RANGE", "操作原因須為 1–200 字")); }
    Ok(reason)
}

/// 與 staff 自行刪帳號共用鎖，避免並行降權／停權移除所有營運人員。
pub(crate) async fn lock_staff_changes(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>) -> Result<(), AppError> {
    // ponytail: 低頻帳號管理共用交易鎖；管理吞吐量成為瓶頸時改為專用權限表的列鎖。
    sqlx::query("SELECT pg_advisory_xact_lock(871024)").execute(&mut **tx).await?;
    Ok(())
}
pub(crate) async fn protect_last_staff(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, id: Uuid) -> Result<(), AppError> {
    let another: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE id<>$1 AND is_staff AND deleted_at IS NULL AND blocked_at IS NULL)")
        .bind(id).fetch_one(&mut **tx).await?;
    if !another { return Err(AppError::problem(409, "LAST_STAFF", "不能移除最後一位有效 staff")); }
    Ok(())
}
async fn staff_transaction(pool: &PgPool, sid: Uuid) -> Result<sqlx::Transaction<'_, sqlx::Postgres>, AppError> {
    let mut tx = pool.begin().await?;
    lock_staff_changes(&mut tx).await?;
    let valid: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE id=$1 AND is_staff AND blocked_at IS NULL AND deleted_at IS NULL)")
        .bind(sid).fetch_one(&mut *tx).await?;
    if !valid { return Err(AppError::problem(403, "STAFF_ONLY", "僅營運人員可使用")); }
    Ok(tx)
}
async fn revoke_sessions(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, id: Uuid) -> Result<(), AppError> {
    sqlx::query("UPDATE sessions SET revoked_at=now() WHERE user_id=$1 AND revoked_at IS NULL").bind(id).execute(&mut **tx).await?;
    Ok(())
}

async fn update_user(State(st): State<AppState>, Staff(sid): Staff, Path(id): Path<Uuid>, Json(b): Json<Value>) -> Result<Json<Value>, AppError> {
    let reason = operation_reason(&b)?;
    for key in ["blocked", "is_staff"] {
        if b.get(key).is_some_and(|v| !v.is_boolean()) { return Err(AppError::invalid(&format!("/{key}"), "TYPE", "須為 boolean")); }
    }
    if b.get("blocked").is_none() && b.get("is_staff").is_none() { return Err(AppError::invalid("/", "REQUIRED", "須提供停權或 staff 設定")); }
    let mut tx = staff_transaction(&st.pool, sid).await?;
    let (staff, blocked): (bool, bool) = sqlx::query_as("SELECT is_staff, blocked_at IS NOT NULL FROM users WHERE id=$1 AND deleted_at IS NULL FOR UPDATE")
        .bind(id).fetch_optional(&mut *tx).await?.ok_or(AppError::NotFound)?;
    let new_staff = b.get("is_staff").and_then(Value::as_bool).unwrap_or(staff);
    let new_blocked = b.get("blocked").and_then(Value::as_bool).unwrap_or(blocked);
    if staff && !blocked && (!new_staff || new_blocked) { protect_last_staff(&mut tx, id).await?; }
    let row: Value = sqlx::query_scalar("UPDATE users SET is_staff=$2, blocked_at=CASE WHEN $3 THEN COALESCE(blocked_at,now()) ELSE NULL END WHERE id=$1
        RETURNING jsonb_build_object('id',id,'is_staff',is_staff,'blocked',blocked_at IS NOT NULL,'blocked_at',blocked_at)")
        .bind(id).bind(new_staff).bind(new_blocked).fetch_one(&mut *tx).await?;
    if new_blocked || staff != new_staff { revoke_sessions(&mut tx, id).await?; }
    audit(&mut *tx, sid, "user.update", "users", Some(id), json!({"reason":reason,"before":{"is_staff":staff,"blocked":blocked},"after":{"is_staff":new_staff,"blocked":new_blocked}})).await?;
    tx.commit().await?;
    Ok(Json(row))
}

async fn unlink_identity(State(st): State<AppState>, Staff(sid): Staff, Path((id, provider)): Path<(Uuid, String)>, Json(b): Json<Value>) -> Result<Json<Value>, AppError> {
    enum_filter(Some(&provider), &["email", "line", "google"], "/provider")?;
    let reason = operation_reason(&b)?;
    let mut tx = staff_transaction(&st.pool, sid).await?;
    let contact: Option<Option<String>> = sqlx::query_scalar("SELECT email FROM users WHERE id=$1 AND deleted_at IS NULL FOR UPDATE").bind(id).fetch_optional(&mut *tx).await?;
    let contact = contact.ok_or(AppError::NotFound)?;
    crate::identities::unlink_locked(&mut tx, id, &provider, contact.as_deref()).await?;
    revoke_sessions(&mut tx, id).await?;
    audit(&mut *tx, sid, "user.identity_unlink", "users", Some(id), json!({"provider":provider,"reason":reason})).await?;
    tx.commit().await?;
    Ok(Json(json!({"unlinked":true})))
}

async fn recover_user(State(st): State<AppState>, Staff(sid): Staff, Path(id): Path<Uuid>, Json(b): Json<Value>) -> Result<(axum::http::StatusCode, Json<Value>), AppError> {
    let reason = operation_reason(&b)?;
    let mut tx = staff_transaction(&st.pool, sid).await?;
    let (email, blocked): (Option<String>, bool) = sqlx::query_as("SELECT email, blocked_at IS NOT NULL FROM users WHERE id=$1 AND deleted_at IS NULL")
        .bind(id).fetch_optional(&mut *tx).await?.ok_or(AppError::NotFound)?;
    if blocked { return Err(AppError::problem(403, "ACCOUNT_BLOCKED", "帳號已停權，請先解除停權")); }
    let email = email.ok_or_else(|| AppError::problem(409, "RECOVERY_UNAVAILABLE", "帳號沒有已驗證 Email，須另行核驗身分"))?;
    // 先取得 OTP 鎖，再鎖 user，與綁定驗證碼共用鎖順序；避免持交易另取 pool 連線。
    let code = crate::auth::issue_otp_in_tx(&mut tx, &email, "reset", None, None, None, None).await?;
    let valid: Option<Uuid> = sqlx::query_scalar("SELECT id FROM users WHERE id=$1 AND email=$2 AND deleted_at IS NULL AND blocked_at IS NULL FOR SHARE")
        .bind(id).bind(&email).fetch_optional(&mut *tx).await?;
    if valid.is_none() { return Err(AppError::problem(409, "RECOVERY_UNAVAILABLE", "帳號狀態已變更，請重新查詢後再試")); }
    // 記錄請求，不記驗證碼；OTP 與稽核一同提交，提交後才寄送。
    audit(&mut *tx, sid, "user.recovery", "users", Some(id), json!({"reason":reason})).await?;
    tx.commit().await?;
    crate::auth::send_otp(&email, code, "reset");
    Ok((axum::http::StatusCode::ACCEPTED, Json(json!({"requested":true}))))
}

#[derive(Deserialize)]
struct AuditQ {
    cursor: Option<Uuid>, limit: Option<i64>, actor_id: Option<Uuid>, action: Option<String>,
    entity: Option<String>, entity_id: Option<Uuid>, from: Option<chrono::NaiveDate>, to: Option<chrono::NaiveDate>,
}
async fn list_audit_logs(State(st): State<AppState>, _s: Staff, Query(q): Query<AuditQ>) -> Result<Response, AppError> {
    if q.from.zip(q.to).is_some_and(|(from,to)| from>to) { return Err(AppError::invalid("/to", "RANGE", "結束日期不可早於開始日期")); }
    let lim = q.limit.unwrap_or(20).clamp(1,100);
    let rows: Vec<(Uuid,Value)> = sqlx::query_as("SELECT a.id,jsonb_build_object('id',a.id,'actor_type',a.actor_type::text,'actor_id',a.actor_id,
        'actor_name',u.display_name,'action',a.action,'entity',a.entity,'entity_id',a.entity_id,'diff',a.diff,'created_at',a.created_at)
        FROM audit_logs a LEFT JOIN users u ON u.id=a.actor_id AND a.actor_type::text IN ('staff','user')
        WHERE ($1::uuid IS NULL OR a.id<$1) AND ($2::uuid IS NULL OR a.actor_id=$2)
          AND ($3::text IS NULL OR a.action=$3) AND ($4::text IS NULL OR a.entity=$4) AND ($5::uuid IS NULL OR a.entity_id=$5)
          AND ($6::date IS NULL OR a.created_at >= ($6::date::timestamp AT TIME ZONE 'Asia/Taipei'))
          AND ($7::date IS NULL OR a.created_at < (($7::date+1)::timestamp AT TIME ZONE 'Asia/Taipei'))
        ORDER BY a.id DESC LIMIT $8")
        .bind(q.cursor).bind(q.actor_id).bind(&q.action).bind(&q.entity).bind(q.entity_id).bind(q.from).bind(q.to).bind(lim+1).fetch_all(&st.pool).await?;
    Ok(page(rows,lim))
}

async fn list_flags(State(st): State<AppState>, _s: Staff) -> Result<Json<Value>, AppError> {
    let rows: Vec<(String, Value, chrono::DateTime<chrono::Utc>)> = sqlx::query_as("SELECT key, value, updated_at FROM system_flags ORDER BY key").fetch_all(&st.pool).await?;
    Ok(Json(json!({ "data": rows.into_iter().map(|(k, v, t)| json!({ "key": k, "value": v, "updated_at": t })).collect::<Vec<_>>() })))
}

/// 白名單 key：目前僅 read_only（boolean）。READ_ONLY_MODE 的 503 攔截不在本模組。
async fn set_flag(State(st): State<AppState>, Staff(sid): Staff, Path(key): Path<String>, Json(b): Json<Value>) -> Result<Json<Value>, AppError> {
    if key != "read_only" { return Err(AppError::NotFound); }
    let v = b.get("value").filter(|v| v.is_boolean()).cloned().ok_or_else(|| AppError::invalid("/value", "INVALID", "value 必須為 boolean"))?;
    let mut tx = st.pool.begin().await?;
    let (updated_at,): (chrono::DateTime<chrono::Utc>,) = sqlx::query_as(
        "INSERT INTO system_flags (key, value) VALUES ($1, $2) ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value RETURNING updated_at")
        .bind(&key).bind(&v).fetch_one(&mut *tx).await?;
    audit(&mut *tx, sid, "system_flag.set", "system_flags", None, json!({ "key": key, "value": v })).await?;
    tx.commit().await?;
    tracing::warn!(key = %key, value = %v, "system flag changed");
    Ok(Json(json!({ "key": key, "value": v, "updated_at": updated_at })))
}

/// 最簡統計：直接聚合現有表，不引入第三方。
pub async fn stats_json(pool: &PgPool) -> Result<Value, sqlx::Error> {
    let (v,): (Value,) = sqlx::query_as(
        "SELECT jsonb_build_object(
           'users', (SELECT count(*) FROM users WHERE deleted_at IS NULL),
           'wishlists', (SELECT count(*) FROM wishlists WHERE deleted_at IS NULL),
           'active_wishlists', (SELECT count(*) FROM wishlists WHERE deleted_at IS NULL AND status = 'active'),
           'hidden_wishlists', (SELECT count(*) FROM wishlists WHERE moderation_status = 'hidden'),
           'claims_by_status', (SELECT COALESCE(jsonb_object_agg(s, n), '{}') FROM (SELECT status::text s, count(*) n FROM claims GROUP BY 1) t),
           'open_reports', (SELECT count(*) FROM content_reports WHERE status = 'open'),
           'notifications_by_status', (SELECT COALESCE(jsonb_object_agg(s, n), '{}') FROM (SELECT status::text s, count(*) n FROM notifications GROUP BY 1) t),
           'audit_last_7d', (SELECT COALESCE(jsonb_object_agg(a, n), '{}') FROM (SELECT action a, count(*) n FROM audit_logs WHERE created_at > now() - interval '7 days' GROUP BY 1) t),
           'flags', (SELECT COALESCE(jsonb_object_agg(key, value), '{}') FROM system_flags))")
        .fetch_one(pool).await?;
    Ok(v)
}

async fn stats(State(st): State<AppState>, _s: Staff) -> Result<Json<Value>, AppError> { Ok(Json(stats_json(&st.pool).await?)) }
