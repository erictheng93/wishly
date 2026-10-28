//! 組織成員與邀請：列表、邀請（連結權杖）、接受、角色/停用。
//! 鎖序：organizations（FOR UPDATE）→ org_members；改成員的 tx 都先鎖組織列，
//! 「至少留一位有效 owner」靠這把鎖串行化（owner 列不另鎖，鎖組織列即足以排他）。
use crate::{
    error::AppError,
    guest::new_token,
    orgs::{audit_user, require_role},
    session::{hash_token, CurrentUser},
    validate, AppState,
};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, get, patch, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

type R<T> = Result<T, AppError>;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/orgs/{id}/members", get(members))
        .route("/orgs/{id}/members/{user_id}", patch(update_member))
        .route("/orgs/{id}/invites", get(invites).post(invite_create))
        .route("/orgs/{id}/invites/{invite_id}", delete(invite_cancel))
        .route("/org-invites/{token}", get(invite_preview))
        .route("/org-invites/{token}/accept", post(invite_accept))
}

const ROLES: [&str; 3] = ["owner", "coordinator", "volunteer"];

fn clean_handle(h: &Option<String>) -> R<Option<String>> {
    match h {
        None => Ok(None),
        Some(s) => {
            let t = validate::text(s, "/handle", false)?;
            if t.chars().count() > 30 {
                return Err(AppError::invalid("/handle", "LENGTH", "長度須在 30 字內"));
            }
            Ok(Some(t).filter(|t| !t.is_empty()))
        }
    }
}

/// 先鎖組織列；不存在 → 404
async fn lock_org(tx: &mut crate::points::Tx<'_>, id: Uuid) -> R<()> {
    sqlx::query_scalar::<_, Uuid>("SELECT id FROM organizations WHERE id=$1 FOR UPDATE")
        .bind(id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(AppError::NotFound)?;
    Ok(())
}

macro_rules! member_sql {
    ($tail:literal) => {
        concat!("SELECT jsonb_build_object('user_id', m.user_id, 'display_name', u.display_name, 'role', m.role::text,
    'handle', m.handle, 'disabled_at', m.disabled_at) FROM org_members m JOIN users u ON u.id = m.user_id ", $tail)
    };
}

async fn members(
    u: CurrentUser,
    State(st): State<AppState>,
    Path(id): Path<Uuid>,
) -> R<Json<Vec<Value>>> {
    require_role(&st.pool, id, u.id, "volunteer").await?;
    let rows = sqlx::query_scalar(member_sql!(
        "WHERE m.org_id=$1 ORDER BY CASE m.role WHEN 'owner' THEN 1 WHEN 'coordinator' THEN 2 ELSE 3 END, m.created_at"))
        .bind(id).fetch_all(&st.pool).await?;
    Ok(Json(rows))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MemberPatch {
    role: Option<String>,
    handle: Option<String>,
    disabled: Option<bool>,
}

async fn update_member(
    u: CurrentUser,
    State(st): State<AppState>,
    Path((id, target)): Path<(Uuid, Uuid)>,
    Json(b): Json<MemberPatch>,
) -> R<Json<Value>> {
    if let Some(r) = &b.role {
        if !ROLES.contains(&r.as_str()) {
            return Err(AppError::invalid("/role", "ENUM", "角色不正確"));
        }
    }
    let handle = clean_handle(&b.handle)?;
    require_role(&st.pool, id, u.id, "owner").await?;
    let mut tx = crate::begin_contended(&st.pool).await?;
    lock_org(&mut tx, id).await?;
    let owners: Vec<Uuid> = sqlx::query_scalar(
        "SELECT user_id FROM org_members WHERE org_id=$1 AND role='owner' AND disabled_at IS NULL",
    )
    .bind(id)
    .fetch_all(&mut *tx)
    .await?;
    let (role, disabled): (String, bool) = sqlx::query_as("SELECT role::text, disabled_at IS NOT NULL FROM org_members WHERE org_id=$1 AND user_id=$2")
        .bind(id).bind(target).fetch_optional(&mut *tx).await?.ok_or(AppError::NotFound)?;
    let new_role = b.role.clone().unwrap_or_else(|| role.clone());
    let new_disabled = b.disabled.unwrap_or(disabled);
    // 目標原本是有效 owner、之後不再是，且他是最後一位 → 擋
    let was_owner = role == "owner" && !disabled;
    let stays_owner = new_role == "owner" && !new_disabled;
    if was_owner && !stays_owner && owners.len() <= 1 {
        return Err(AppError::problem(
            409,
            "LAST_OWNER",
            "組織至少需要一位有效的擁有者",
        ));
    }
    // 等鎖期間操作者可能已被降級 / 停用
    if !owners.contains(&u.id) {
        return Err(AppError::problem(
            403,
            "ORG_ROLE_REQUIRED",
            "此操作需要更高的組織角色",
        ));
    }
    sqlx::query("UPDATE org_members SET role=$3::org_role, handle=CASE WHEN $4 THEN $5 ELSE handle END,
                   disabled_at=CASE WHEN $6 THEN COALESCE(disabled_at, now()) ELSE NULL END WHERE org_id=$1 AND user_id=$2")
        .bind(id).bind(target).bind(&new_role).bind(b.handle.is_some()).bind(&handle).bind(new_disabled)
        .execute(&mut *tx).await?;
    audit_user(&mut *tx, u.id, "org.member.update", "org_member", Some(target),
        json!({"org_id": id, "role": [role, new_role], "disabled": [disabled, new_disabled], "handle": b.handle})).await?;
    let v: Value = sqlx::query_scalar(member_sql!("WHERE m.org_id=$1 AND m.user_id=$2"))
        .bind(id)
        .bind(target)
        .fetch_one(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Json(v))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InviteBody {
    email: String,
    role: String,
    handle: Option<String>,
}

async fn invite_create(
    u: CurrentUser,
    State(st): State<AppState>,
    Path(id): Path<Uuid>,
    Json(b): Json<InviteBody>,
) -> R<(StatusCode, Json<Value>)> {
    require_role(&st.pool, id, u.id, "owner").await?;
    let email = crate::auth::norm_email(&b.email)?;
    if !ROLES.contains(&b.role.as_str()) {
        return Err(AppError::invalid("/role", "ENUM", "角色不正確"));
    }
    let handle = clean_handle(&b.handle)?;
    let (token, h) = new_token();
    let url = format!("{}/org/invite/{token}", crate::auth::app_url());
    let mut tx = st.pool.begin().await?;
    let invite: Uuid = sqlx::query_scalar(
        "INSERT INTO org_invites (org_id, email, role, handle, token_hash, invited_by, expires_at)
        VALUES ($1,$2,$3::org_role,$4,$5,$6, now() + interval '7 days') RETURNING id",
    )
    .bind(id)
    .bind(&email)
    .bind(&b.role)
    .bind(&handle)
    .bind(h)
    .bind(u.id)
    .fetch_one(&mut *tx)
    .await?;
    // 只通知既有使用者；寄給信箱的邀請連結由邀請人自行轉交
    let existing: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM users WHERE email=$1 AND deleted_at IS NULL")
            .bind(&email)
            .fetch_optional(&mut *tx)
            .await?;
    if let Some(uid) = existing {
        let name: String = sqlx::query_scalar("SELECT display_name FROM organizations WHERE id=$1")
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO notifications (user_id, channel, kind, payload) VALUES ($1,'email','org.invite',$2)")
            .bind(uid).bind(json!({"org_id": id, "org_display_name": name, "role": b.role, "url": url})).execute(&mut *tx).await?;
    }
    audit_user(
        &mut *tx,
        u.id,
        "org.invite.create",
        "org_invite",
        Some(invite),
        json!({"org_id": id, "email": email, "role": b.role}),
    )
    .await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(json!({"id": invite, "url": url}))))
}

async fn invites(
    u: CurrentUser,
    State(st): State<AppState>,
    Path(id): Path<Uuid>,
) -> R<Json<Vec<Value>>> {
    require_role(&st.pool, id, u.id, "owner").await?;
    let rows = sqlx::query_scalar("SELECT jsonb_build_object('id', id, 'email', email, 'role', role::text, 'handle', handle, 'expires_at', expires_at,
        'accepted_at', accepted_at, 'cancelled_at', cancelled_at, 'created_at', created_at) FROM org_invites WHERE org_id=$1 ORDER BY created_at DESC")
        .bind(id).fetch_all(&st.pool).await?;
    Ok(Json(rows))
}

async fn invite_cancel(
    u: CurrentUser,
    State(st): State<AppState>,
    Path((id, invite)): Path<(Uuid, Uuid)>,
) -> R<StatusCode> {
    require_role(&st.pool, id, u.id, "owner").await?;
    let mut tx = st.pool.begin().await?;
    let n = sqlx::query("UPDATE org_invites SET cancelled_at=now() WHERE id=$1 AND org_id=$2 AND cancelled_at IS NULL AND accepted_at IS NULL")
        .bind(invite).bind(id).execute(&mut *tx).await?.rows_affected();
    if n == 0 {
        return Err(AppError::NotFound);
    }
    audit_user(
        &mut *tx,
        u.id,
        "org.invite.cancel",
        "org_invite",
        Some(invite),
        json!({"org_id": id}),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

fn expired() -> AppError {
    AppError::problem(410, "INVITE_EXPIRED", "邀請已過期或已取消")
}

type Inv = (Uuid, Uuid, String, String, Option<String>, String);

macro_rules! find_sql {
    ($tail:literal) => {
        concat!(
            "SELECT i.id, i.org_id, i.email, i.role::text, i.handle, o.display_name,
           i.cancelled_at IS NULL AND i.accepted_at IS NULL AND i.expires_at > now()
         FROM org_invites i JOIN organizations o ON o.id = i.org_id WHERE i.token_hash=$1",
            $tail
        )
    };
}

/// 有效邀請（未取消、未接受、未過期）；未知權杖 404，其餘 410。
async fn find(ex: impl sqlx::PgExecutor<'_>, token: &str, lock: bool) -> R<Inv> {
    let (id, org, email, role, handle, name, ok): (
        Uuid,
        Uuid,
        String,
        String,
        Option<String>,
        String,
        bool,
    ) = sqlx::query_as(if lock {
        find_sql!(" FOR UPDATE OF i")
    } else {
        find_sql!("")
    })
    .bind(hash_token(token))
    .fetch_optional(ex)
    .await?
    .ok_or(AppError::NotFound)?;
    if !ok {
        return Err(expired());
    }
    Ok((id, org, email, role, handle, name))
}

async fn invite_preview(
    _u: CurrentUser,
    State(st): State<AppState>,
    Path(token): Path<String>,
) -> R<Json<Value>> {
    let (_, org, email, role, _, name) = find(&st.pool, &token, false).await?;
    Ok(Json(
        json!({"org_id": org, "org_display_name": name, "role": role, "email": email}),
    ))
}

async fn invite_accept(
    u: CurrentUser,
    State(st): State<AppState>,
    Path(token): Path<String>,
) -> R<Json<Value>> {
    let mut tx = crate::begin_contended(&st.pool).await?;
    let org = find(&mut *tx, &token, false).await?.1;
    lock_org(&mut tx, org).await?;
    let (invite, _, email, role, handle, _) = find(&mut *tx, &token, true).await?;
    // 已驗證 email：email 登入（provider_uid = 小寫 email）或同 email 的 Google 登入
    let verified: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM auth_identities WHERE user_id=$1
        AND ((provider='email' AND provider_uid=$2) OR (provider='google' AND email=$2)))",
    )
    .bind(u.id)
    .bind(&email)
    .fetch_one(&mut *tx)
    .await?;
    if !verified {
        return Err(AppError::Extra {
            status: 403,
            code: "INVITE_EMAIL_MISMATCH",
            detail: "此邀請寄給另一個 email".into(),
            extra: json!({"email": email}),
        });
    }
    let disabled: Option<bool> = sqlx::query_scalar(
        "SELECT disabled_at IS NOT NULL FROM org_members WHERE org_id=$1 AND user_id=$2",
    )
    .bind(org)
    .bind(u.id)
    .fetch_optional(&mut *tx)
    .await?;
    match disabled {
        Some(false) => return Err(AppError::problem(409, "ALREADY_MEMBER", "你已是此組織成員")),
        Some(true) => {
            sqlx::query("UPDATE org_members SET disabled_at=NULL, role=$3::org_role, handle=COALESCE($4, handle) WHERE org_id=$1 AND user_id=$2")
            .bind(org).bind(u.id).bind(&role).bind(&handle).execute(&mut *tx).await?;
        }
        None => {
            sqlx::query("INSERT INTO org_members (org_id, user_id, role, handle) VALUES ($1,$2,$3::org_role,$4)")
            .bind(org).bind(u.id).bind(&role).bind(&handle).execute(&mut *tx).await?;
        }
    }
    sqlx::query("UPDATE org_invites SET accepted_at=now() WHERE id=$1")
        .bind(invite)
        .execute(&mut *tx)
        .await?;
    audit_user(
        &mut *tx,
        u.id,
        "org.invite.accept",
        "org_invite",
        Some(invite),
        json!({"org_id": org, "role": role}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"org_id": org, "role": role})))
}
