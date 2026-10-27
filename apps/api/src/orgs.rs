//! 組織申請（spec 2026-10-27-org-relief-registration-design）：草稿、送審、撤回與角色守衛。
//! 鎖序：organizations（FOR UPDATE）→ org_members。
use crate::{error::AppError, session::CurrentUser, validate, wishlists::new_slug, AppState};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::PgExecutor;
use uuid::Uuid;

type R<T> = Result<T, AppError>;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/orgs", post(create))
        .route("/orgs/{id}", get(show).patch(patch))
        .route("/orgs/{id}/submit", post(submit))
        .route("/orgs/{id}/withdraw", post(withdraw))
}

/// 未停用成員的角色；非成員 → None
pub async fn member_role<'e>(ex: impl PgExecutor<'e>, org: Uuid, user: Uuid) -> R<Option<String>> {
    Ok(sqlx::query_scalar("SELECT role::text FROM org_members WHERE org_id=$1 AND user_id=$2 AND disabled_at IS NULL")
        .bind(org).bind(user).fetch_optional(ex).await?)
}

fn rank(r: &str) -> u8 {
    match r {
        "volunteer" => 1,
        "coordinator" => 2,
        "owner" => 3,
        _ => 4, // 未知角色要求一律視為最嚴
    }
}

/// 非成員 / 已停用 → 404；角色低於 `min` → 403 ORG_ROLE_REQUIRED。回傳實際角色。
pub async fn require_role<'e>(ex: impl PgExecutor<'e>, org: Uuid, user: Uuid, min: &str) -> R<String> {
    let role = member_role(ex, org, user).await?.ok_or(AppError::NotFound)?;
    if rank(&role) < rank(min) {
        return Err(AppError::problem(403, "ORG_ROLE_REQUIRED", "此操作需要更高的組織角色"));
    }
    Ok(role)
}

/// 使用者操作稽核；`diff` 須自帶 `org_id`。
pub async fn audit_user<'e>(ex: impl PgExecutor<'e>, user: Uuid, action: &str, entity: &str, id: Option<Uuid>, diff: Value) -> R<()> {
    sqlx::query("INSERT INTO audit_logs (actor_type, actor_id, action, entity, entity_id, diff) VALUES ('user', $1, $2, $3, $4, $5)")
        .bind(user).bind(action).bind(entity).bind(id).bind(diff).execute(ex).await?;
    Ok(())
}

fn map_unique(e: sqlx::Error) -> AppError {
    if let sqlx::Error::Database(d) = &e {
        match d.constraint() {
            Some("organizations_tax_id_live") => return AppError::problem(409, "TAX_ID_TAKEN", "此統一編號已有申請中或已通過的組織"),
            Some("organizations_one_draft") => return AppError::problem(409, "ORG_DRAFT_EXISTS", "你已有一份草稿組織，請先完成或撤回"),
            _ => {}
        }
    }
    e.into()
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct Fields {
    legal_name: Option<String>,
    display_name: Option<String>,
    org_type: Option<String>,
    tax_id: Option<String>,
    registration_no: Option<String>,
    authority: Option<String>,
    website: Option<String>,
    intro: Option<String>,
    rep_name: Option<String>,
    contact_name: Option<String>,
    contact_title: Option<String>,
    contact_phone: Option<String>,
    public_email: Option<String>,
    public_phone: Option<String>,
}

/// 驗證並正規化；回傳有帶的欄位名（供 needs_changes 鎖欄位用）。給了就不可為空（本版不支援清空欄位）。
fn clean(mut f: Fields) -> R<(Fields, Vec<&'static str>)> {
    let mut names = vec![];
    macro_rules! chk {
        ($($n:ident: $max:expr, $multi:expr);* $(;)?) => {$(
            if let Some(s) = &f.$n {
                let p = concat!("/", stringify!($n));
                let t = validate::text(s, p, $multi)?;
                if t.is_empty() || t.chars().count() > $max {
                    return Err(AppError::invalid(p, "LENGTH", "不可為空且長度須在限制內"));
                }
                f.$n = Some(t);
                names.push(stringify!($n));
            }
        )*};
    }
    chk!(legal_name: 100, false; display_name: 50, false; org_type: 20, false; tax_id: 8, false;
         registration_no: 100, false; authority: 100, false; website: 300, false; intro: 2000, true;
         rep_name: 50, false; contact_name: 50, false; contact_title: 50, false; contact_phone: 30, false;
         public_email: 200, false; public_phone: 30, false);
    if let Some(t) = &f.org_type {
        if !["association", "foundation", "religious", "other"].contains(&t.as_str()) {
            return Err(AppError::invalid("/org_type", "ENUM", "組織類型不正確"));
        }
    }
    if let Some(t) = &f.tax_id {
        if !validate::tax_id_ok(t) {
            return Err(AppError::invalid("/tax_id", "TAX_ID_CHECKSUM", "統一編號檢查碼錯誤"));
        }
    }
    Ok((f, names))
}

fn seal_phone(f: &Fields) -> Option<Vec<u8>> {
    f.contact_phone.as_ref().map(|p| crate::sealed::seal(p.as_bytes()))
}

/// 欄位名與表相同；contact_phone 只給 owner（解密）。
async fn org_json<'e>(ex: impl PgExecutor<'e>, id: Uuid, role: &str) -> R<Value> {
    let (mut v, enc): (Value, Option<Vec<u8>>) =
        sqlx::query_as("SELECT to_jsonb(o) - 'contact_phone_enc', contact_phone_enc FROM organizations o WHERE id=$1")
            .bind(id).fetch_one(ex).await?;
    v["role"] = json!(role);
    if role == "owner" {
        v["contact_phone"] = json!(enc.as_deref().and_then(crate::sealed::open_str));
    }
    Ok(v)
}

async fn create(u: CurrentUser, State(st): State<AppState>, Json(b): Json<Fields>) -> R<(StatusCode, Json<Value>)> {
    for (n, v) in [("legal_name", &b.legal_name), ("display_name", &b.display_name)] {
        if v.is_none() {
            return Err(AppError::invalid(&format!("/{n}"), "REQUIRED", "必填"));
        }
    }
    let (f, _) = clean(b)?;
    for _ in 0..5 {
        let mut tx = crate::begin_contended(&st.pool).await?;
        let r = sqlx::query_scalar::<_, Uuid>(
            "INSERT INTO organizations (slug, legal_name, display_name, org_type, tax_id, registration_no, authority, website, intro,
               rep_name, contact_name, contact_title, contact_phone_enc, public_email, public_phone, created_by)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16) RETURNING id")
            .bind(new_slug()).bind(&f.legal_name).bind(&f.display_name).bind(&f.org_type).bind(&f.tax_id).bind(&f.registration_no)
            .bind(&f.authority).bind(&f.website).bind(&f.intro).bind(&f.rep_name).bind(&f.contact_name).bind(&f.contact_title)
            .bind(seal_phone(&f)).bind(&f.public_email).bind(&f.public_phone).bind(u.id)
            .fetch_one(&mut *tx).await;
        let id = match r {
            Ok(id) => id,
            Err(sqlx::Error::Database(e)) if e.constraint() == Some("organizations_slug_key") => continue,
            Err(e) => return Err(map_unique(e)),
        };
        sqlx::query("INSERT INTO org_members (org_id, user_id, role) VALUES ($1,$2,'owner')").bind(id).bind(u.id).execute(&mut *tx).await?;
        audit_user(&mut *tx, u.id, "org.create", "organizations", Some(id), json!({"org_id": id})).await?;
        let v = org_json(&mut *tx, id, "owner").await?;
        tx.commit().await?;
        return Ok((StatusCode::CREATED, Json(v)));
    }
    Err(AppError::problem(500, "INTERNAL_ERROR", "無法產生代碼，請重試"))
}

async fn show(u: CurrentUser, State(st): State<AppState>, Path(id): Path<Uuid>) -> R<Json<Value>> {
    let role = member_role(&st.pool, id, u.id).await?.ok_or(AppError::NotFound)?;
    Ok(Json(org_json(&st.pool, id, &role).await?))
}

/// 鎖住組織列（先於 org_members），回傳目前狀態。
async fn lock(tx: &mut crate::points::Tx<'_>, id: Uuid) -> R<(String, Vec<String>, Option<bool>)> {
    sqlx::query_as("SELECT verification_status, flagged_fields, resubmit_after > now() FROM organizations WHERE id=$1 FOR UPDATE")
        .bind(id).fetch_optional(&mut **tx).await?.ok_or(AppError::NotFound)
}

async fn patch(u: CurrentUser, State(st): State<AppState>, Path(id): Path<Uuid>, Json(b): Json<Fields>) -> R<Json<Value>> {
    let mut tx = crate::begin_contended(&st.pool).await?;
    let (status, flagged, too_early) = lock(&mut tx, id).await?;
    require_role(&mut *tx, id, u.id, "owner").await?;
    let (f, names) = clean(b)?;
    let reopen = match status.as_str() {
        "draft" => false,
        "needs_changes" => {
            if let Some(n) = names.iter().find(|n| !flagged.iter().any(|x| x == *n)) {
                return Err(AppError::invalid(&format!("/{n}"), "FIELD_LOCKED", "此欄位未被標記，不可修改"));
            }
            false
        }
        "rejected" => {
            if too_early == Some(true) {
                return Err(AppError::problem(409, "RESUBMIT_TOO_EARLY", "尚未到可重新申請的時間"));
            }
            true
        }
        _ => return Err(AppError::problem(409, "ORG_NOT_EDITABLE", "目前狀態不可修改")),
    };
    sqlx::query(
        "UPDATE organizations SET legal_name=COALESCE($2,legal_name), display_name=COALESCE($3,display_name), org_type=COALESCE($4,org_type),
           tax_id=COALESCE($5,tax_id), registration_no=COALESCE($6,registration_no), authority=COALESCE($7,authority),
           website=COALESCE($8,website), intro=COALESCE($9,intro), rep_name=COALESCE($10,rep_name), contact_name=COALESCE($11,contact_name),
           contact_title=COALESCE($12,contact_title), contact_phone_enc=COALESCE($13,contact_phone_enc),
           public_email=COALESCE($14,public_email), public_phone=COALESCE($15,public_phone),
           verification_status=CASE WHEN $16 THEN 'draft' ELSE verification_status END,
           flagged_fields=CASE WHEN $16 THEN '{}' ELSE flagged_fields END
         WHERE id=$1")
        .bind(id).bind(&f.legal_name).bind(&f.display_name).bind(&f.org_type).bind(&f.tax_id).bind(&f.registration_no)
        .bind(&f.authority).bind(&f.website).bind(&f.intro).bind(&f.rep_name).bind(&f.contact_name).bind(&f.contact_title)
        .bind(seal_phone(&f)).bind(&f.public_email).bind(&f.public_phone).bind(reopen)
        .execute(&mut *tx).await.map_err(map_unique)?;
    audit_user(&mut *tx, u.id, "org.update", "organizations", Some(id), json!({"org_id": id, "fields": names})).await?;
    let v = org_json(&mut *tx, id, "owner").await?;
    tx.commit().await?;
    Ok(Json(v))
}

async fn submit(u: CurrentUser, State(st): State<AppState>, Path(id): Path<Uuid>) -> R<Json<Value>> {
    let mut tx = crate::begin_contended(&st.pool).await?;
    let (status, _, _) = lock(&mut tx, id).await?;
    require_role(&mut *tx, id, u.id, "owner").await?;
    if !matches!(status.as_str(), "draft" | "needs_changes") {
        return Err(AppError::problem(409, "ORG_NOT_EDITABLE", "目前狀態不可送審"));
    }
    let row = org_json(&mut *tx, id, "owner").await?;
    for (col, ptr) in [("org_type", "org_type"), ("tax_id", "tax_id"), ("registration_no", "registration_no"), ("authority", "authority"),
                       ("rep_name", "rep_name"), ("contact_name", "contact_name"), ("contact_phone", "contact_phone"), ("public_email", "public_email")] {
        if row[col].is_null() {
            return Err(AppError::invalid(&format!("/{ptr}"), "REQUIRED", "送審前必填"));
        }
    }
    let docs: i64 = sqlx::query_scalar("SELECT count(*) FROM org_documents WHERE org_id=$1 AND kind='registration'").bind(id).fetch_one(&mut *tx).await?;
    if docs == 0 {
        return Err(AppError::invalid("/documents", "REQUIRED", "請至少上傳一份立案文件"));
    }
    let permits: i64 = sqlx::query_scalar("SELECT count(*) FROM org_permits WHERE org_id=$1").bind(id).fetch_one(&mut *tx).await?;
    if permits == 0 {
        return Err(AppError::invalid("/permits", "REQUIRED", "請至少新增一張勸募許可"));
    }
    sqlx::query("UPDATE organizations SET verification_status='pending', submitted_at=now(), flagged_fields='{}' WHERE id=$1")
        .bind(id).execute(&mut *tx).await.map_err(map_unique)?;
    audit_user(&mut *tx, u.id, "org.submit", "organizations", Some(id), json!({"org_id": id})).await?;
    let v = org_json(&mut *tx, id, "owner").await?;
    tx.commit().await?;
    Ok(Json(v))
}

async fn withdraw(u: CurrentUser, State(st): State<AppState>, Path(id): Path<Uuid>) -> R<Json<Value>> {
    let mut tx = crate::begin_contended(&st.pool).await?;
    let (status, _, _) = lock(&mut tx, id).await?;
    require_role(&mut *tx, id, u.id, "owner").await?;
    if status != "pending" {
        return Err(AppError::problem(409, "ORG_NOT_EDITABLE", "只有審核中的申請可撤回"));
    }
    sqlx::query("UPDATE organizations SET verification_status='draft', submitted_at=NULL WHERE id=$1")
        .bind(id).execute(&mut *tx).await.map_err(map_unique)?;
    audit_user(&mut *tx, u.id, "org.withdraw", "organizations", Some(id), json!({"org_id": id})).await?;
    let v = org_json(&mut *tx, id, "owner").await?;
    tx.commit().await?;
    Ok(Json(v))
}
