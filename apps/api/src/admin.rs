//! 營運後台：STAFF 守衛 + 清單/使用者搜尋 + system-flags + 簡易統計。
//! 檢舉佇列與下架在 reports.rs（同樣使用 `Staff`）。
use crate::{error::AppError, session::CurrentUser, AppState};
use axum::{
    extract::{FromRequestParts, Path, Query, State},
    http::{header, request::Parts, HeaderValue, StatusCode},
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
        .route(
            "/admin/users/{id}/identities/{provider}",
            delete(unlink_identity),
        )
        .route("/admin/users/{id}/recovery", post(recover_user))
        .route("/admin/users/{id}/mfa", delete(reset_mfa))
        .route("/admin/audit-logs", get(list_audit_logs))
        .route("/admin/system-flags", get(list_flags))
        .route("/admin/system-flags/{key}", put(set_flag))
        .route("/admin/notifications/requeue", post(requeue_notifications))
        .route("/admin/stats", get(stats))
        .route("/admin/analytics", get(analytics))
}

/// 已登入且 is_staff；已啟用 TOTP、或 system_flags.staff_mfa_required 開啟（#99，預設關）時，此 session 須在 12 小時內通過驗證。
/// 否則 401 / 403 STAFF_ONLY / MFA_REQUIRED。驗證 12 小時後失效須重驗（session.rs）。
pub struct Staff(pub Uuid);

impl FromRequestParts<AppState> for Staff {
    type Rejection = AppError;
    async fn from_request_parts(parts: &mut Parts, st: &AppState) -> Result<Self, AppError> {
        let u = CurrentUser::from_request_parts(parts, st).await?;
        require_staff(&u)?;
        Ok(Staff(u.id))
    }
}

/// `Staff` 守衛的判斷；已持有 `CurrentUser`、只有部分分支需要 staff 時直接呼叫。
pub fn require_staff(u: &CurrentUser) -> Result<(), AppError> {
    if !u.is_staff {
        Err(AppError::problem(403, "STAFF_ONLY", "僅營運人員可使用"))
    } else if !u.mfa && (u.mfa_enabled || u.mfa_required) {
        Err(AppError::problem(403, "MFA_REQUIRED", "請先完成兩步驟驗證"))
    } else {
        Ok(())
    }
}

pub async fn audit<'e, E: sqlx::PgExecutor<'e>>(
    ex: E,
    staff: Uuid,
    action: &str,
    entity: &str,
    entity_id: Option<Uuid>,
    diff: Value,
) -> Result<(), AppError> {
    sqlx::query("INSERT INTO audit_logs (actor_type, actor_id, action, entity, entity_id, diff) VALUES ('staff', $1, $2, $3, $4, $5)")
        .bind(staff).bind(action).bind(entity).bind(entity_id).bind(diff).execute(ex).await?;
    Ok(())
}

#[derive(Deserialize)]
pub struct PageQ {
    pub cursor: Option<Uuid>,
    pub limit: Option<i64>,
    pub q: Option<String>,
    pub status: Option<String>,
    pub moderation_status: Option<String>,
}

pub fn limit(q: &PageQ) -> i64 {
    q.limit.unwrap_or(20).clamp(1, 100)
}

/// rows 為 limit+1 筆 (id, json)；cursor 為最後一筆 id（uuid v7 時間序，opaque 給前端）。
pub fn page(mut rows: Vec<(Uuid, Value)>, limit: i64) -> Response {
    let next = if rows.len() as i64 > limit {
        rows.truncate(limit as usize);
        rows.last().map(|r| r.0)
    } else {
        None
    };
    let mut res = Json(
        json!({ "data": rows.into_iter().map(|r| r.1).collect::<Vec<_>>(), "next_cursor": next }),
    )
    .into_response();
    res.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    res
}

async fn list_wishlists(
    State(st): State<AppState>,
    Staff(sid): Staff,
    Query(q): Query<PageQ>,
) -> Result<Response, AppError> {
    if let Some(m) = &q.moderation_status {
        if m != "ok" && m != "hidden" {
            return Err(AppError::invalid(
                "/moderation_status",
                "INVALID",
                "ok | hidden",
            ));
        }
    }
    enum_filter(
        q.status.as_deref(),
        &["draft", "active", "closed", "archived"],
        "/status",
    )?;
    let qs = q.q.as_deref().map(str::trim).filter(|s| !s.is_empty());
    if let Some(s) = qs {
        audit(
            &st.pool,
            sid,
            "admin.search",
            "wishlists",
            None,
            json!({ "q": s }),
        )
        .await?;
    }
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
    cursor: Option<Uuid>,
    limit: Option<i64>,
    q: Option<String>,
    status: Option<String>,
    provider: Option<String>,
}

fn enum_filter(value: Option<&str>, allowed: &[&str], pointer: &str) -> Result<(), AppError> {
    if value.is_some_and(|v| !allowed.contains(&v)) {
        return Err(AppError::invalid(pointer, "ENUM", "篩選值不正確"));
    }
    Ok(())
}

async fn list_users(
    State(st): State<AppState>,
    Staff(sid): Staff,
    Query(q): Query<UserQ>,
) -> Result<Response, AppError> {
    let lim = q.limit.unwrap_or(20).clamp(1, 100);
    enum_filter(
        q.provider.as_deref(),
        &["email", "line", "google"],
        "/provider",
    )?;
    enum_filter(
        q.status.as_deref(),
        &["active", "blocked", "deleted", "staff"],
        "/status",
    )?;
    let qs = q.q.as_deref().map(str::trim).filter(|s| !s.is_empty());
    if let Some(s) = qs {
        audit(
            &st.pool,
            sid,
            "admin.search",
            "users",
            None,
            json!({ "q": s }),
        )
        .await?;
    }
    let qid = qs.and_then(|s| s.parse::<Uuid>().ok());
    let rows: Vec<(Uuid, Value)> = sqlx::query_as(
        "SELECT u.id, jsonb_build_object('id', u.id, 'display_name', u.display_name, 'handle', u.handle, 'email', u.email, 'is_staff', u.is_staff, 'mfa_enabled', u.totp_enabled_at IS NOT NULL,
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
    if !(1..=200).contains(&reason.chars().count()) {
        return Err(AppError::invalid(
            "/reason",
            "RANGE",
            "操作原因須為 1–200 字",
        ));
    }
    Ok(reason)
}

/// 與 staff 自行刪帳號共用鎖，避免並行降權／停權移除所有營運人員。
pub(crate) async fn lock_staff_changes(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<(), AppError> {
    // ponytail: 低頻帳號管理共用交易鎖；管理吞吐量成為瓶頸時改為專用權限表的列鎖。
    sqlx::query("SELECT pg_advisory_xact_lock(871024)")
        .execute(&mut **tx)
        .await?;
    Ok(())
}
pub(crate) async fn protect_last_staff(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: Uuid,
) -> Result<(), AppError> {
    let another: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE id<>$1 AND is_staff AND deleted_at IS NULL AND blocked_at IS NULL)")
        .bind(id).fetch_one(&mut **tx).await?;
    if !another {
        return Err(AppError::problem(
            409,
            "LAST_STAFF",
            "不能移除最後一位有效 staff",
        ));
    }
    Ok(())
}
async fn staff_transaction(
    pool: &PgPool,
    sid: Uuid,
) -> Result<sqlx::Transaction<'_, sqlx::Postgres>, AppError> {
    let mut tx = pool.begin().await?;
    lock_staff_changes(&mut tx).await?;
    let valid: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE id=$1 AND is_staff AND blocked_at IS NULL AND deleted_at IS NULL)")
        .bind(sid).fetch_one(&mut *tx).await?;
    if !valid {
        return Err(AppError::problem(403, "STAFF_ONLY", "僅營運人員可使用"));
    }
    Ok(tx)
}
async fn revoke_sessions(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: Uuid,
) -> Result<(), AppError> {
    sqlx::query("UPDATE sessions SET revoked_at=now() WHERE user_id=$1 AND revoked_at IS NULL")
        .bind(id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// 把最近 7 天寄送失敗（非地址錯誤）的通知重新排入佇列。
async fn requeue_notifications(
    State(st): State<AppState>,
    Staff(sid): Staff,
) -> Result<Json<Value>, AppError> {
    let mut tx = st.pool.begin().await?;
    let n = sqlx::query("UPDATE notifications SET status = 'pending', attempts = 0, scheduled_at = now()
                          WHERE status = 'failed' AND scheduled_at > now() - interval '7 days' AND last_error IS DISTINCT FROM 'bad address'")
        .execute(&mut *tx).await?.rows_affected();
    audit(
        &mut *tx,
        sid,
        "notification.requeue",
        "notifications",
        None,
        json!({ "requeued": n }),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({ "requeued": n })))
}

async fn update_user(
    State(st): State<AppState>,
    Staff(sid): Staff,
    Path(id): Path<Uuid>,
    Json(b): Json<Value>,
) -> Result<Json<Value>, AppError> {
    let reason = operation_reason(&b)?;
    for key in ["blocked", "is_staff"] {
        if b.get(key).is_some_and(|v| !v.is_boolean()) {
            return Err(AppError::invalid(
                &format!("/{key}"),
                "TYPE",
                "須為 boolean",
            ));
        }
    }
    if b.get("blocked").is_none() && b.get("is_staff").is_none() {
        return Err(AppError::invalid(
            "/",
            "REQUIRED",
            "須提供停權或 staff 設定",
        ));
    }
    let mut tx = staff_transaction(&st.pool, sid).await?;
    let (staff, blocked): (bool, bool) = sqlx::query_as("SELECT is_staff, blocked_at IS NOT NULL FROM users WHERE id=$1 AND deleted_at IS NULL FOR UPDATE")
        .bind(id).fetch_optional(&mut *tx).await?.ok_or(AppError::NotFound)?;
    let new_staff = b.get("is_staff").and_then(Value::as_bool).unwrap_or(staff);
    let new_blocked = b.get("blocked").and_then(Value::as_bool).unwrap_or(blocked);
    if staff && !blocked && (!new_staff || new_blocked) {
        protect_last_staff(&mut tx, id).await?;
    }
    let row: Value = sqlx::query_scalar("UPDATE users SET is_staff=$2, blocked_at=CASE WHEN $3 THEN COALESCE(blocked_at,now()) ELSE NULL END WHERE id=$1
        RETURNING jsonb_build_object('id',id,'is_staff',is_staff,'blocked',blocked_at IS NOT NULL,'blocked_at',blocked_at)")
        .bind(id).bind(new_staff).bind(new_blocked).fetch_one(&mut *tx).await?;
    if new_blocked || staff != new_staff {
        revoke_sessions(&mut tx, id).await?;
    }
    // 新停權：該使用者公開清單的邊緣副本要清掉（內容已改回 404）
    let purge: Vec<String> = if new_blocked && !blocked {
        sqlx::query_scalar("SELECT slug::text FROM wishlists WHERE owner_id=$1 AND deleted_at IS NULL AND visibility='public'")
            .bind(id).fetch_all(&mut *tx).await?
    } else {
        vec![]
    };
    if new_blocked != blocked {
        crate::notify::enqueue(
            &mut *tx,
            id,
            if new_blocked {
                "account.suspended"
            } else {
                "account.restored"
            },
            json!({}),
        )
        .await?;
    }
    audit(&mut *tx, sid, "user.update", "users", Some(id), json!({"reason":reason,"before":{"is_staff":staff,"blocked":blocked},"after":{"is_staff":new_staff,"blocked":new_blocked}})).await?;
    tx.commit().await?;
    if !purge.is_empty() {
        crate::cdn::purge_wishlists(purge.iter().map(|s| s.trim().to_string()).collect());
    }
    Ok(Json(row))
}

async fn unlink_identity(
    State(st): State<AppState>,
    Staff(sid): Staff,
    Path((id, provider)): Path<(Uuid, String)>,
    Json(b): Json<Value>,
) -> Result<Json<Value>, AppError> {
    enum_filter(Some(&provider), &["email", "line", "google"], "/provider")?;
    let reason = operation_reason(&b)?;
    let mut tx = staff_transaction(&st.pool, sid).await?;
    let contact: Option<Option<String>> =
        sqlx::query_scalar("SELECT email FROM users WHERE id=$1 AND deleted_at IS NULL FOR UPDATE")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?;
    let contact = contact.ok_or(AppError::NotFound)?;
    crate::identities::unlink_locked(&mut tx, id, &provider, contact.as_deref()).await?;
    revoke_sessions(&mut tx, id).await?;
    audit(
        &mut *tx,
        sid,
        "user.identity_unlink",
        "users",
        Some(id),
        json!({"provider":provider,"reason":reason}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"unlinked":true})))
}

/// 營運人員遺失驗證器時由另一位營運人員重設；撤銷其 session，下次登入重新設定。
async fn reset_mfa(
    State(st): State<AppState>,
    Staff(sid): Staff,
    Path(id): Path<Uuid>,
    Json(b): Json<Value>,
) -> Result<StatusCode, AppError> {
    let reason = operation_reason(&b)?;
    if id == sid {
        // 自己重設等於繞過第二因素；遺失驗證器要找另一位營運人員
        return Err(AppError::problem(
            409,
            "MFA_RESET_SELF",
            "不能重設自己的兩步驟驗證",
        ));
    }
    let mut tx = staff_transaction(&st.pool, sid).await?;
    let email: Option<String> = sqlx::query_scalar("UPDATE users SET totp_secret=NULL, totp_enabled_at=NULL, totp_last_step=NULL WHERE id=$1 AND deleted_at IS NULL RETURNING email")
        .bind(id).fetch_optional(&mut *tx).await?.ok_or(AppError::NotFound)?;
    sqlx::query("DELETE FROM totp_recovery_codes WHERE user_id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    revoke_sessions(&mut tx, id).await?;
    // 通知當事人，被盜用的帳號替他人關掉第二因素時當事人能立刻發現
    crate::identities::notify(&mut tx, id, "auth.mfa_reset", "totp", email.as_deref()).await?;
    audit(
        &mut *tx,
        sid,
        "user.mfa_reset",
        "users",
        Some(id),
        json!({"reason":reason}),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn recover_user(
    State(st): State<AppState>,
    Staff(sid): Staff,
    Path(id): Path<Uuid>,
    Json(b): Json<Value>,
) -> Result<(axum::http::StatusCode, Json<Value>), AppError> {
    let reason = operation_reason(&b)?;
    let mut tx = staff_transaction(&st.pool, sid).await?;
    let (email, blocked): (Option<String>, bool) = sqlx::query_as(
        "SELECT email, blocked_at IS NOT NULL FROM users WHERE id=$1 AND deleted_at IS NULL",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(AppError::NotFound)?;
    if blocked {
        return Err(AppError::problem(
            403,
            "ACCOUNT_BLOCKED",
            "帳號已停權，請先解除停權",
        ));
    }
    let email = email.ok_or_else(|| {
        AppError::problem(
            409,
            "RECOVERY_UNAVAILABLE",
            "帳號沒有已驗證 Email，須另行核驗身分",
        )
    })?;
    // 先取得 OTP 鎖，再鎖 user，與綁定驗證碼共用鎖順序；避免持交易另取 pool 連線。
    let (_, code) =
        crate::auth::issue_otp_in_tx(&mut tx, &email, "reset", None, None, None, None, false)
            .await?;
    let valid: Option<Uuid> = sqlx::query_scalar("SELECT id FROM users WHERE id=$1 AND email=$2 AND deleted_at IS NULL AND blocked_at IS NULL FOR SHARE")
        .bind(id).bind(&email).fetch_optional(&mut *tx).await?;
    if valid.is_none() {
        return Err(AppError::problem(
            409,
            "RECOVERY_UNAVAILABLE",
            "帳號狀態已變更，請重新查詢後再試",
        ));
    }
    // 記錄請求，不記驗證碼；OTP 與稽核一同提交，提交後才寄送。
    audit(
        &mut *tx,
        sid,
        "user.recovery",
        "users",
        Some(id),
        json!({"reason":reason}),
    )
    .await?;
    tx.commit().await?;
    crate::auth::send_otp(&email, code, "reset");
    Ok((
        axum::http::StatusCode::ACCEPTED,
        Json(json!({"requested":true})),
    ))
}

#[derive(Deserialize)]
struct AuditQ {
    cursor: Option<Uuid>,
    limit: Option<i64>,
    actor_id: Option<Uuid>,
    action: Option<String>,
    entity: Option<String>,
    entity_id: Option<Uuid>,
    from: Option<chrono::NaiveDate>,
    to: Option<chrono::NaiveDate>,
}
async fn list_audit_logs(
    State(st): State<AppState>,
    _s: Staff,
    Query(q): Query<AuditQ>,
) -> Result<Response, AppError> {
    if q.from.zip(q.to).is_some_and(|(from, to)| from > to) {
        return Err(AppError::invalid(
            "/to",
            "RANGE",
            "結束日期不可早於開始日期",
        ));
    }
    let lim = q.limit.unwrap_or(20).clamp(1, 100);
    let rows: Vec<(Uuid,Value)> = sqlx::query_as("SELECT a.id,jsonb_build_object('id',a.id,'actor_type',a.actor_type::text,'actor_id',a.actor_id,
        'actor_name',u.display_name,'action',a.action,'entity',a.entity,'entity_id',a.entity_id,'diff',a.diff,'created_at',a.created_at)
        FROM audit_logs a LEFT JOIN users u ON u.id=a.actor_id AND a.actor_type::text IN ('staff','user')
        WHERE ($1::uuid IS NULL OR a.id<$1) AND ($2::uuid IS NULL OR a.actor_id=$2)
          AND ($3::text IS NULL OR a.action=$3) AND ($4::text IS NULL OR a.entity=$4) AND ($5::uuid IS NULL OR a.entity_id=$5)
          AND ($6::date IS NULL OR a.created_at >= ($6::date::timestamp AT TIME ZONE 'Asia/Taipei'))
          AND ($7::date IS NULL OR a.created_at < (($7::date+1)::timestamp AT TIME ZONE 'Asia/Taipei'))
        ORDER BY a.id DESC LIMIT $8")
        .bind(q.cursor).bind(q.actor_id).bind(&q.action).bind(&q.entity).bind(q.entity_id).bind(q.from).bind(q.to).bind(lim+1).fetch_all(&st.pool).await?;
    Ok(page(rows, lim))
}

async fn list_flags(State(st): State<AppState>, _s: Staff) -> Result<Json<Value>, AppError> {
    let rows: Vec<(String, Value, chrono::DateTime<chrono::Utc>)> =
        sqlx::query_as("SELECT key, value, updated_at FROM system_flags ORDER BY key")
            .fetch_all(&st.pool)
            .await?;
    Ok(Json(
        json!({ "data": rows.into_iter().map(|(k, v, t)| json!({ "key": k, "value": v, "updated_at": t })).collect::<Vec<_>>() }),
    ))
}

/// 白名單 key：read_only、staff_mfa_required（boolean）。READ_ONLY_MODE 的 503 攔截不在本模組。
async fn set_flag(
    State(st): State<AppState>,
    u: CurrentUser,
    Path(key): Path<String>,
    Json(b): Json<Value>,
) -> Result<Json<Value>, AppError> {
    require_staff(&u)?;
    let sid = u.id;
    if key != "read_only" && key != "staff_mfa_required" {
        return Err(AppError::NotFound);
    }
    let v = b
        .get("value")
        .filter(|v| v.is_boolean())
        .cloned()
        .ok_or_else(|| AppError::invalid("/value", "INVALID", "value 必須為 boolean"))?;
    if key == "staff_mfa_required" && v == json!(true) {
        // 開啟前確認不會把人鎖在外面：自己此 session 已驗證，且至少 2 位營運人員已設定（有人遺失驗證器時還有人能重設）
        if !u.mfa {
            return Err(AppError::problem(403, "MFA_REQUIRED", "請先完成兩步驟驗證"));
        }
        let ready: i64 = sqlx::query_scalar("SELECT count(*) FROM users WHERE is_staff AND totp_enabled_at IS NOT NULL AND deleted_at IS NULL AND blocked_at IS NULL")
            .fetch_one(&st.pool).await?;
        if ready < 2 {
            return Err(AppError::problem(
                409,
                "STAFF_MFA_TOO_FEW",
                "至少 2 位營運人員設定兩步驟驗證後才能開啟強制",
            ));
        }
    }
    let mut tx = st.pool.begin().await?;
    let (updated_at,): (chrono::DateTime<chrono::Utc>,) = sqlx::query_as(
        "INSERT INTO system_flags (key, value) VALUES ($1, $2) ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value RETURNING updated_at")
        .bind(&key).bind(&v).fetch_one(&mut *tx).await?;
    audit(
        &mut *tx,
        sid,
        "system_flag.set",
        "system_flags",
        None,
        json!({ "key": key, "value": v }),
    )
    .await?;
    tx.commit().await?;
    tracing::warn!(key = %key, value = %v, "system flag changed");
    Ok(Json(
        json!({ "key": key, "value": v, "updated_at": updated_at }),
    ))
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
           'staff_mfa', (SELECT count(*) FROM users WHERE is_staff AND totp_enabled_at IS NOT NULL AND deleted_at IS NULL AND blocked_at IS NULL),
           'notifications_by_status', (SELECT COALESCE(jsonb_object_agg(s, n), '{}') FROM (SELECT status::text s, count(*) n FROM notifications GROUP BY 1) t),
           'audit_last_7d', (SELECT COALESCE(jsonb_object_agg(a, n), '{}') FROM (SELECT action a, count(*) n FROM audit_logs WHERE created_at > now() - interval '7 days' GROUP BY 1) t),
           'flags', (SELECT COALESCE(jsonb_object_agg(key, value), '{}') FROM system_flags))")
        .fetch_one(pool).await?;
    Ok(v)
}

async fn stats(State(st): State<AppState>, _s: Staff) -> Result<Json<Value>, AppError> {
    Ok(Json(stats_json(&st.pool).await?))
}

#[derive(Deserialize)]
struct AnalyticsQ {
    days: Option<i32>,
}

/// 營運分析：日界線 Asia/Taipei；distribution / birthday_month 小樣本（1..4）輸出 null（k=5）。
async fn analytics(
    State(st): State<AppState>,
    _s: Staff,
    Query(q): Query<AnalyticsQ>,
) -> Result<Response, AppError> {
    let days = q.days.unwrap_or(30);
    if !(7..=365).contains(&days) {
        return Err(AppError::invalid(
            "/days",
            "OUT_OF_RANGE",
            "days 須介於 7 到 365",
        ));
    }
    let (v,): (Value,) = sqlx::query_as(
        "WITH b AS (SELECT (now() AT TIME ZONE 'Asia/Taipei')::date AS today),
         r AS (SELECT today - ($1 - 1) AS d0, today FROM b),
         -- 區間起點（含）的 timestamptz
         lo AS (SELECT d0::timestamp AT TIME ZONE 'Asia/Taipei' AS t FROM r),
         cu AS (SELECT * FROM users WHERE deleted_at IS NULL AND NOT is_staff AND NOT EXISTS (SELECT 1 FROM wishlists d WHERE d.owner_id = users.id AND d.slug::text LIKE 'demo%')),
         wl AS (SELECT w.* FROM wishlists w JOIN cu ON cu.id = w.owner_id WHERE w.deleted_at IS NULL),
         cohort AS (SELECT id, created_at FROM cu WHERE created_at >= (SELECT t FROM lo)),
         -- 留存 cohort：台北註冊日落在區間內
         ccu AS (SELECT id, (created_at AT TIME ZONE 'Asia/Taipei')::date AS s FROM cohort),
         pu AS (SELECT * FROM cu WHERE anonymized_at IS NULL),
         rc AS (SELECT * FROM claims WHERE created_at >= (SELECT t FROM lo)),
         -- 每日趨勢：各表只掃一次再依台北日期分組
         ev(m, d) AS (
           SELECT 'signups', (created_at AT TIME ZONE 'Asia/Taipei')::date FROM cohort
           UNION ALL SELECT 'wishlists', (created_at AT TIME ZONE 'Asia/Taipei')::date FROM wl WHERE created_at >= (SELECT t FROM lo)
           UNION ALL SELECT 'claims', (created_at AT TIME ZONE 'Asia/Taipei')::date FROM rc
           UNION ALL SELECT 'new_guests', (created_at AT TIME ZONE 'Asia/Taipei')::date FROM guests WHERE created_at >= (SELECT t FROM lo)),
         daily AS (
           SELECT gs::date d,
             count(*) FILTER (WHERE ev.m = 'signups') signups, count(*) FILTER (WHERE ev.m = 'wishlists') wishlists,
             count(*) FILTER (WHERE ev.m = 'claims') claims, count(*) FILTER (WHERE ev.m = 'new_guests') new_guests,
             (SELECT count(DISTINCT a.user_id) FROM user_activity_days a JOIN cu ON cu.id = a.user_id WHERE a.day = gs::date) active_users
           FROM r CROSS JOIN generate_series(r.d0, r.today, interval '1 day') gs LEFT JOIN ev ON ev.d = gs::date GROUP BY gs),
         dist(kind, k, n) AS (
           SELECT 'provider', ai.provider::text, count(DISTINCT ai.user_id) FROM auth_identities ai JOIN cu ON cu.id = ai.user_id GROUP BY 2
           UNION ALL SELECT 'locale', locale, count(*) FROM cu GROUP BY 2
           UNION ALL SELECT 'wishlist_type', type::text, count(*) FROM wl GROUP BY 2
           UNION ALL SELECT 'wishlist_visibility', visibility::text, count(*) FROM wl GROUP BY 2
           UNION ALL SELECT 'birthday_visibility', birthday_visibility::text, count(*) FROM pu WHERE birthday IS NOT NULL GROUP BY 2),
         bm AS (
           SELECT lpad(m::text, 2, '0') k, count(pu.id) n
           FROM generate_series(1, 12) m LEFT JOIN pu ON substr(pu.birthday, 1, 2) = lpad(m::text, 2, '0') GROUP BY 1)
         SELECT jsonb_build_object(
           'range', (SELECT jsonb_build_object('from', d0, 'to', today, 'days', $1) FROM r),
           'totals', jsonb_build_object(
             'users', (SELECT count(*) FROM cu),
             'guests', (SELECT count(*) FROM guests WHERE deleted_at IS NULL),
             'wishlists', (SELECT count(*) FROM wl),
             'friendships', (SELECT count(*) FROM friendships)),
           'active', (SELECT jsonb_object_agg('d' || n, (
               SELECT count(DISTINCT a.user_id) FROM user_activity_days a JOIN cu ON cu.id = a.user_id
               WHERE a.day > (SELECT today FROM b) - n)) FROM unnest(ARRAY[1, 7, 30]) n),
           'daily', (SELECT jsonb_agg(jsonb_build_object('date', d, 'signups', signups, 'wishlists', wishlists,
               'claims', claims, 'new_guests', new_guests, 'active_users', active_users) ORDER BY d) FROM daily),
           -- 留存：區間內註冊且視窗已完整結束者為 cohort；視窗內任一天活躍即 retained
           'retention', (SELECT jsonb_object_agg('d' || w.k, jsonb_build_object(
               'cohort', (SELECT count(*) FROM ccu WHERE ccu.s + w.hi < (SELECT today FROM b)),
               'retained', (SELECT count(*) FROM ccu WHERE ccu.s + w.hi < (SELECT today FROM b) AND EXISTS (
                  SELECT 1 FROM user_activity_days a WHERE a.user_id = ccu.id AND a.day BETWEEN ccu.s + w.lo AND ccu.s + w.hi))))
             FROM (VALUES (1, 1, 1), (7, 7, 13), (30, 30, 59)) w(k, lo, hi)),
           'funnel', jsonb_build_object(
             'signed_up', (SELECT count(*) FROM cohort),
             'created_list', (SELECT count(*) FROM cohort WHERE EXISTS (SELECT 1 FROM wl WHERE wl.owner_id = cohort.id)),
             'published_list', (SELECT count(*) FROM cohort WHERE EXISTS (SELECT 1 FROM wl WHERE wl.owner_id = cohort.id AND wl.status <> 'draft')),
             'received_claim', (SELECT count(*) FROM cohort WHERE EXISTS (
                SELECT 1 FROM wl JOIN wishlist_items i ON i.wishlist_id = wl.id
                WHERE wl.owner_id = cohort.id AND (EXISTS (SELECT 1 FROM claims c WHERE c.item_id = i.id)
                  OR EXISTS (SELECT 1 FROM contributions k WHERE k.item_id = i.id))))),
           'claims', jsonb_build_object(
             'by_status', (SELECT COALESCE(jsonb_object_agg(s, n), '{}') FROM (SELECT status::text s, count(*) n FROM rc GROUP BY 1) t),
             'by_actor', jsonb_build_object(
               'guest', (SELECT count(*) FROM rc WHERE guest_id IS NOT NULL),
               'user', (SELECT count(*) FROM rc WHERE user_id IS NOT NULL))),
           'trust', jsonb_build_object(
             'reports', (SELECT count(*) FROM content_reports WHERE created_at >= (SELECT t FROM lo)),
             'reports_by_reason', (SELECT COALESCE(jsonb_object_agg(s, n), '{}') FROM (SELECT reason::text s, count(*) n FROM content_reports WHERE created_at >= (SELECT t FROM lo) GROUP BY 1) t),
             'login_failures', (SELECT count(*) FROM login_failures WHERE created_at >= (SELECT t FROM lo) AND pending_until IS NULL)),
           'profile', jsonb_build_object(
             'users', (SELECT count(*) FROM pu),
             'with_bio', (SELECT count(*) FROM pu WHERE bio IS NOT NULL),
             'with_birthday', (SELECT count(*) FROM pu WHERE birthday IS NOT NULL),
             'with_avatar', (SELECT count(*) FROM pu WHERE avatar_key IS NOT NULL),
             'upcoming_birthdays_30d', (SELECT count(*) FROM pu WHERE birthday IN (
                SELECT to_char((SELECT today FROM b) + i, 'MM-DD') FROM generate_series(0, 29) i))),
           'birthday_month', (SELECT jsonb_object_agg(k, CASE WHEN n BETWEEN 1 AND 4 THEN NULL ELSE to_jsonb(n) END) FROM bm),
           'distribution', (SELECT jsonb_object_agg(kind, o) FROM (
               SELECT kind, jsonb_object_agg(k, CASE WHEN n < 5 THEN NULL ELSE to_jsonb(n) END) o FROM dist GROUP BY kind) t))",
    )
    .bind(days)
    .fetch_one(&st.pool)
    .await?;
    let mut v = v;
    // 空桶整組省略後，前端仍需要五個鍵
    for k in [
        "provider",
        "locale",
        "wishlist_type",
        "wishlist_visibility",
        "birthday_visibility",
    ] {
        if v["distribution"].get(k).is_none() {
            v["distribution"][k] = json!({});
        }
    }
    let mut res = Json(v).into_response();
    res.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(res)
}
