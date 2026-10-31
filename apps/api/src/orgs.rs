//! 組織申請（spec 2026-10-27-org-relief-registration-design）：草稿、送審、撤回與角色守衛。
//! 鎖序：organizations（FOR UPDATE）→ org_members。
use crate::{error::AppError, session::CurrentUser, validate, wishlists::new_slug, AppState};
use axum::{
    body::Bytes,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::PgExecutor;
use uuid::Uuid;

const MAX_DOC: usize = 10 * 1024 * 1024;

type R<T> = Result<T, AppError>;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/orgs", post(create))
        .route("/orgs/{id}", get(show).patch(patch))
        .route("/orgs/{id}/submit", post(submit))
        .route("/orgs/{id}/withdraw", post(withdraw))
        // 上限多 1 byte：恰 10MB+1 由 upload 回 422 /content_length，更大才是 413
        .route(
            "/orgs/{id}/documents",
            post(upload).layer(DefaultBodyLimit::max(MAX_DOC + 1)),
        )
        .route(
            "/orgs/{id}/documents/{doc}",
            get(download).delete(delete_doc),
        )
        .route("/orgs/{id}/permits", get(list_permits).post(add_permit))
        .route("/orgs/{id}/permits/{pid}/apply", post(apply_permit))
}

/// 未停用成員的角色；非成員 → None
pub async fn member_role<'e>(ex: impl PgExecutor<'e>, org: Uuid, user: Uuid) -> R<Option<String>> {
    Ok(sqlx::query_scalar(
        "SELECT role::text FROM org_members WHERE org_id=$1 AND user_id=$2 AND disabled_at IS NULL",
    )
    .bind(org)
    .bind(user)
    .fetch_optional(ex)
    .await?)
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
pub async fn require_role<'e>(
    ex: impl PgExecutor<'e>,
    org: Uuid,
    user: Uuid,
    min: &str,
) -> R<String> {
    let role = member_role(ex, org, user)
        .await?
        .ok_or(AppError::NotFound)?;
    if rank(&role) < rank(min) {
        return Err(AppError::problem(
            403,
            "ORG_ROLE_REQUIRED",
            "此操作需要更高的組織角色",
        ));
    }
    Ok(role)
}

/// 使用者操作稽核；`diff` 須自帶 `org_id`。
pub async fn audit_user<'e>(
    ex: impl PgExecutor<'e>,
    user: Uuid,
    action: &str,
    entity: &str,
    id: Option<Uuid>,
    diff: Value,
) -> R<()> {
    sqlx::query("INSERT INTO audit_logs (actor_type, actor_id, action, entity, entity_id, diff) VALUES ('user', $1, $2, $3, $4, $5)")
        .bind(user).bind(action).bind(entity).bind(id).bind(diff).execute(ex).await?;
    Ok(())
}

fn map_unique(e: sqlx::Error) -> AppError {
    if let sqlx::Error::Database(d) = &e {
        match d.constraint() {
            Some("organizations_tax_id_live") => {
                return AppError::problem(409, "TAX_ID_TAKEN", "此統一編號已有申請中或已通過的組織")
            }
            Some("organizations_one_draft") => {
                return AppError::problem(
                    409,
                    "ORG_DRAFT_EXISTS",
                    "你已有一份草稿組織，請先完成或撤回",
                )
            }
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
            return Err(AppError::invalid(
                "/tax_id",
                "TAX_ID_CHECKSUM",
                "統一編號檢查碼錯誤",
            ));
        }
    }
    Ok((f, names))
}

fn seal_phone(f: &Fields) -> Option<Vec<u8>> {
    f.contact_phone
        .as_ref()
        .map(|p| crate::sealed::seal(p.as_bytes()))
}

/// 欄位名與表相同；contact_phone 只給 owner（解密）。
async fn org_json<'e>(ex: impl PgExecutor<'e>, id: Uuid, role: &str) -> R<Value> {
    let (mut v, enc): (Value, Option<Vec<u8>>) =
        sqlx::query_as("SELECT (to_jsonb(o) - 'contact_phone_enc') || jsonb_build_object('documents', (SELECT coalesce(jsonb_agg(jsonb_build_object(
               'id', d.id, 'kind', d.kind, 'content_type', d.content_type, 'size_bytes', d.size_bytes, 'created_at', d.created_at) ORDER BY d.created_at, d.id), '[]'::jsonb)
             FROM org_documents d WHERE d.org_id = o.id)), contact_phone_enc FROM organizations o WHERE id=$1")
            .bind(id).fetch_one(ex).await?;
    v["role"] = json!(role);
    if role == "owner" {
        v["contact_phone"] = json!(enc.as_deref().and_then(crate::sealed::open_str));
    }
    Ok(v)
}

async fn create(
    u: CurrentUser,
    State(st): State<AppState>,
    Json(b): Json<Fields>,
) -> R<(StatusCode, Json<Value>)> {
    for (n, v) in [
        ("legal_name", &b.legal_name),
        ("display_name", &b.display_name),
    ] {
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
            Err(sqlx::Error::Database(e)) if e.constraint() == Some("organizations_slug_key") => {
                continue
            }
            Err(e) => return Err(map_unique(e)),
        };
        sqlx::query("INSERT INTO org_members (org_id, user_id, role) VALUES ($1,$2,'owner')")
            .bind(id)
            .bind(u.id)
            .execute(&mut *tx)
            .await?;
        audit_user(
            &mut *tx,
            u.id,
            "org.create",
            "organizations",
            Some(id),
            json!({"org_id": id}),
        )
        .await?;
        let v = org_json(&mut *tx, id, "owner").await?;
        tx.commit().await?;
        return Ok((StatusCode::CREATED, Json(v)));
    }
    Err(AppError::problem(
        500,
        "INTERNAL_ERROR",
        "無法產生代碼，請重試",
    ))
}

async fn show(u: CurrentUser, State(st): State<AppState>, Path(id): Path<Uuid>) -> R<Json<Value>> {
    let role = member_role(&st.pool, id, u.id)
        .await?
        .ok_or(AppError::NotFound)?;
    Ok(Json(org_json(&st.pool, id, &role).await?))
}

/// 鎖住組織列（先於 org_members），回傳目前狀態。
async fn lock(tx: &mut crate::points::Tx<'_>, id: Uuid) -> R<(String, Vec<String>, Option<bool>)> {
    sqlx::query_as("SELECT verification_status, flagged_fields, resubmit_after > now() FROM organizations WHERE id=$1 FOR UPDATE")
        .bind(id).fetch_optional(&mut **tx).await?.ok_or(AppError::NotFound)
}

async fn patch(
    u: CurrentUser,
    State(st): State<AppState>,
    Path(id): Path<Uuid>,
    Json(b): Json<Fields>,
) -> R<Json<Value>> {
    let mut tx = crate::begin_contended(&st.pool).await?;
    let (status, flagged, too_early) = lock(&mut tx, id).await?;
    require_role(&mut *tx, id, u.id, "owner").await?;
    let (f, names) = clean(b)?;
    let reopen = match status.as_str() {
        "draft" => false,
        "needs_changes" => {
            if let Some(n) = names.iter().find(|n| !flagged.iter().any(|x| x == *n)) {
                return Err(AppError::invalid(
                    &format!("/{n}"),
                    "FIELD_LOCKED",
                    "此欄位未被標記，不可修改",
                ));
            }
            false
        }
        "rejected" => {
            if too_early == Some(true) {
                return Err(AppError::problem(
                    409,
                    "RESUBMIT_TOO_EARLY",
                    "尚未到可重新申請的時間",
                ));
            }
            true
        }
        _ => {
            return Err(AppError::problem(
                409,
                "ORG_NOT_EDITABLE",
                "目前狀態不可修改",
            ))
        }
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
    audit_user(
        &mut *tx,
        u.id,
        "org.update",
        "organizations",
        Some(id),
        json!({"org_id": id, "fields": names}),
    )
    .await?;
    let v = org_json(&mut *tx, id, "owner").await?;
    tx.commit().await?;
    Ok(Json(v))
}

async fn submit(
    u: CurrentUser,
    State(st): State<AppState>,
    Path(id): Path<Uuid>,
) -> R<Json<Value>> {
    let mut tx = crate::begin_contended(&st.pool).await?;
    let (status, _, _) = lock(&mut tx, id).await?;
    require_role(&mut *tx, id, u.id, "owner").await?;
    if !matches!(status.as_str(), "draft" | "needs_changes") {
        return Err(AppError::problem(
            409,
            "ORG_NOT_EDITABLE",
            "目前狀態不可送審",
        ));
    }
    let row = org_json(&mut *tx, id, "owner").await?;
    for (col, ptr) in [
        ("org_type", "org_type"),
        ("tax_id", "tax_id"),
        ("registration_no", "registration_no"),
        ("authority", "authority"),
        ("rep_name", "rep_name"),
        ("contact_name", "contact_name"),
        ("contact_phone", "contact_phone"),
        ("public_email", "public_email"),
    ] {
        if row[col].is_null() {
            return Err(AppError::invalid(
                &format!("/{ptr}"),
                "REQUIRED",
                "送審前必填",
            ));
        }
    }
    let docs: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM org_documents WHERE org_id=$1 AND kind='registration'",
    )
    .bind(id)
    .fetch_one(&mut *tx)
    .await?;
    if docs == 0 {
        return Err(AppError::invalid(
            "/documents",
            "REQUIRED",
            "請至少上傳一份立案文件",
        ));
    }
    let permits: i64 = sqlx::query_scalar("SELECT count(*) FROM org_permits WHERE org_id=$1")
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
    if permits == 0 {
        return Err(AppError::invalid(
            "/permits",
            "REQUIRED",
            "請至少新增一張勸募許可",
        ));
    }
    sqlx::query("UPDATE organizations SET verification_status='pending', submitted_at=now(), flagged_fields='{}', review_note=NULL WHERE id=$1")
        .bind(id).execute(&mut *tx).await.map_err(map_unique)?;
    audit_user(
        &mut *tx,
        u.id,
        "org.submit",
        "organizations",
        Some(id),
        json!({"org_id": id}),
    )
    .await?;
    let v = org_json(&mut *tx, id, "owner").await?;
    tx.commit().await?;
    Ok(Json(v))
}

async fn withdraw(
    u: CurrentUser,
    State(st): State<AppState>,
    Path(id): Path<Uuid>,
) -> R<Json<Value>> {
    let mut tx = crate::begin_contended(&st.pool).await?;
    let (status, _, _) = lock(&mut tx, id).await?;
    require_role(&mut *tx, id, u.id, "owner").await?;
    if status != "pending" {
        return Err(AppError::problem(
            409,
            "ORG_NOT_EDITABLE",
            "只有審核中的申請可撤回",
        ));
    }
    sqlx::query(
        "UPDATE organizations SET verification_status='draft', submitted_at=NULL WHERE id=$1",
    )
    .bind(id)
    .execute(&mut *tx)
    .await
    .map_err(map_unique)?;
    audit_user(
        &mut *tx,
        u.id,
        "org.withdraw",
        "organizations",
        Some(id),
        json!({"org_id": id}),
    )
    .await?;
    let v = org_json(&mut *tx, id, "owner").await?;
    tx.commit().await?;
    Ok(Json(v))
}

// ---------- 文件與勸募許可 ----------
#[derive(Deserialize)]
struct KindQ {
    kind: String,
}

/// permit 類（文件與許可）已驗證組織也可追加；立案文件只在草稿 / 待補正。
fn only_editable(status: &str) -> R<()> {
    only_for(status, false)
}
fn only_for(status: &str, permit: bool) -> R<()> {
    if matches!(status, "draft" | "needs_changes") || (permit && status == "verified") {
        Ok(())
    } else {
        Err(AppError::problem(
            409,
            "ORG_NOT_EDITABLE",
            "目前狀態不可修改",
        ))
    }
}

/// 原始檔案做請求本體；先加密再存，bucket 內不留明文。
async fn upload(
    u: CurrentUser,
    State(st): State<AppState>,
    Path(id): Path<Uuid>,
    Query(q): Query<KindQ>,
    headers: HeaderMap,
    body: Bytes,
) -> R<(StatusCode, Json<Value>)> {
    require_role(&st.pool, id, u.id, "owner").await?;
    if !["registration", "permit"].contains(&q.kind.as_str()) {
        return Err(AppError::invalid(
            "/kind",
            "ENUM",
            "kind 必須為 registration 或 permit",
        ));
    }
    let ct = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !["application/pdf", "image/jpeg", "image/png"].contains(&ct) {
        return Err(AppError::invalid(
            "/content_type",
            "ENUM",
            "只允許 PDF、JPEG、PNG",
        ));
    }
    if body.is_empty() || body.len() > MAX_DOC {
        return Err(AppError::invalid(
            "/content_length",
            "RANGE",
            "檔案大小須介於 1 byte 與 10 MB",
        ));
    }
    // Content-Type 由用戶端決定（前端依副檔名推得）：再比對檔頭，改副檔名的任意檔案不會混進審核文件
    let magic: &[u8] = match ct {
        "application/pdf" => b"%PDF-",
        "image/jpeg" => &[0xFF, 0xD8, 0xFF],
        _ => b"\x89PNG\r\n\x1a\n",
    };
    if !body.starts_with(magic) {
        return Err(AppError::invalid(
            "/content_type",
            "MISMATCH",
            "檔案內容與格式不符，請上傳 PDF、JPEG 或 PNG",
        ));
    }
    let permit = q.kind == "permit";
    let early: String =
        sqlx::query_scalar("SELECT verification_status FROM organizations WHERE id=$1")
            .bind(id)
            .fetch_one(&st.pool)
            .await?;
    only_for(&early, permit)?; // 省一次無謂的 S3 寫入；權威判斷在下方鎖內
    crate::ratelimit::check(&st.pool, &format!("org-doc:{}", u.id), 30, 3600).await?;
    let key = format!("org-docs/{id}/{}", Uuid::new_v4());
    crate::uploads::put_object(&key, crate::sealed::seal(&body), "application/octet-stream")
        .await?;
    let r: R<Value> = async {
        let mut tx = crate::begin_contended(&st.pool).await?;
        let (status, _, _) = lock(&mut tx, id).await?;
        require_role(&mut *tx, id, u.id, "owner").await?;
        only_for(&status, permit)?;
        let (doc,): (Uuid,) = sqlx::query_as("INSERT INTO org_documents (org_id, kind, object_key, content_type, size_bytes, uploaded_by) VALUES ($1,$2,$3,$4,$5,$6) RETURNING id")
            .bind(id).bind(&q.kind).bind(&key).bind(ct).bind(body.len() as i32).bind(u.id).fetch_one(&mut *tx).await?;
        audit_user(&mut *tx, u.id, "org.document.upload", "org_documents", Some(doc), json!({"org_id": id, "kind": q.kind})).await?;
        tx.commit().await?;
        Ok(json!({"id": doc, "kind": q.kind, "content_type": ct, "size_bytes": body.len()}))
    }.await;
    match r {
        Ok(v) => Ok((StatusCode::CREATED, Json(v))),
        Err(e) => {
            let _ = crate::uploads::delete_object(&key).await; // 寫庫失敗：不留孤兒密文
            Err(e)
        }
    }
}

async fn download(
    u: CurrentUser,
    State(st): State<AppState>,
    Path((id, doc)): Path<(Uuid, Uuid)>,
) -> R<Response> {
    let role = member_role(&st.pool, id, u.id).await?;
    let member_ok = role
        .as_deref()
        .is_some_and(|r| rank(r) >= rank("coordinator"));
    if !member_ok {
        if u.is_staff {
            crate::admin::require_staff(&u)?;
        } else if role.is_some() {
            return Err(AppError::problem(
                403,
                "ORG_ROLE_REQUIRED",
                "此操作需要更高的組織角色",
            ));
        } else {
            return Err(AppError::NotFound);
        }
    }
    let (key, ct): (String, String) = sqlx::query_as(
        "SELECT object_key, content_type FROM org_documents WHERE id=$1 AND org_id=$2",
    )
    .bind(doc)
    .bind(id)
    .fetch_optional(&st.pool)
    .await?
    .ok_or(AppError::NotFound)?;
    let plain = crate::sealed::open(&crate::uploads::get_object(&key).await?)
        .ok_or_else(|| AppError::problem(500, "INTERNAL_ERROR", "文件無法解密"))?;
    // 營運審核時的下載記為 staff，依 actor_type 統計營運讀取敏感文件才不會漏
    if member_ok {
        audit_user(
            &st.pool,
            u.id,
            "org.document.download",
            "org_documents",
            Some(doc),
            json!({"org_id": id}),
        )
        .await?;
    } else {
        crate::admin::audit(
            &st.pool,
            u.id,
            "org.document.download",
            "org_documents",
            Some(doc),
            json!({"org_id": id}),
        )
        .await?;
    }
    Ok((
        [
            (header::CONTENT_TYPE, ct),
            (header::CONTENT_DISPOSITION, "attachment".into()),
            (header::CACHE_CONTROL, "no-store".into()),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff".into()),
        ],
        plain,
    )
        .into_response())
}

async fn delete_doc(
    u: CurrentUser,
    State(st): State<AppState>,
    Path((id, doc)): Path<(Uuid, Uuid)>,
) -> R<StatusCode> {
    let mut tx = crate::begin_contended(&st.pool).await?;
    let (status, _, _) = lock(&mut tx, id).await?;
    require_role(&mut *tx, id, u.id, "owner").await?;
    only_editable(&status)?;
    let used: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM org_permits WHERE document_id=$1)")
            .bind(doc)
            .fetch_one(&mut *tx)
            .await?;
    if used {
        return Err(AppError::problem(
            409,
            "DOCUMENT_IN_USE",
            "此文件已被勸募許可引用",
        ));
    }
    let key: String = sqlx::query_scalar(
        "DELETE FROM org_documents WHERE id=$1 AND org_id=$2 RETURNING object_key",
    )
    .bind(doc)
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(AppError::NotFound)?;
    audit_user(
        &mut *tx,
        u.id,
        "org.document.delete",
        "org_documents",
        Some(doc),
        json!({"org_id": id}),
    )
    .await?;
    tx.commit().await?;
    if let Err(e) = crate::uploads::delete_object(&key).await {
        tracing::warn!(?e, key, "組織文件物件刪除失敗，留下孤兒密文");
    }
    Ok(StatusCode::NO_CONTENT)
}

const PERMIT_COLS: &str = "jsonb_build_object('id', id, 'permit_no', permit_no, 'valid_from', valid_from, 'valid_to', valid_to,
    'status', status, 'review_note', review_note, 'document_id', document_id, 'created_at', created_at)";

async fn list_permits(
    u: CurrentUser,
    State(st): State<AppState>,
    Path(id): Path<Uuid>,
) -> R<Json<Value>> {
    require_role(&st.pool, id, u.id, "volunteer").await?;
    let rows: Vec<Value> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT {PERMIT_COLS} FROM org_permits WHERE org_id=$1 ORDER BY created_at, id"
    )))
    .bind(id)
    .fetch_all(&st.pool)
    .await?;
    Ok(Json(Value::Array(rows)))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PermitIn {
    permit_no: String,
    valid_from: chrono::NaiveDate,
    valid_to: chrono::NaiveDate,
    document_id: Uuid,
}

async fn add_permit(
    u: CurrentUser,
    State(st): State<AppState>,
    Path(id): Path<Uuid>,
    Json(b): Json<PermitIn>,
) -> R<(StatusCode, Json<Value>)> {
    let permit_no = validate::text(&b.permit_no, "/permit_no", false)?;
    if permit_no.is_empty() || permit_no.chars().count() > 60 {
        return Err(AppError::invalid(
            "/permit_no",
            "LENGTH",
            "不可為空且最多 60 字",
        ));
    }
    if b.valid_to < b.valid_from {
        return Err(AppError::invalid(
            "/valid_to",
            "RANGE",
            "截止日不可早於起始日",
        ));
    }
    crate::ratelimit::check(&st.pool, &format!("org-permit:{}", u.id), 20, 3600).await?;
    let mut tx = crate::begin_contended(&st.pool).await?;
    let (status, _, _) = lock(&mut tx, id).await?; // 與刪文件互斥
    require_role(&mut *tx, id, u.id, "owner").await?;
    only_for(&status, true)?;
    let ok: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM org_documents WHERE id=$1 AND org_id=$2)")
            .bind(b.document_id)
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
    if !ok {
        return Err(AppError::invalid(
            "/document_id",
            "NOT_FOUND",
            "文件不存在或不屬於此組織",
        ));
    }
    let v: Value = sqlx::query_scalar(sqlx::AssertSqlSafe(format!("INSERT INTO org_permits (org_id, permit_no, valid_from, valid_to, document_id) VALUES ($1,$2,$3,$4,$5)
        RETURNING {PERMIT_COLS}")))
        .bind(id).bind(permit_no).bind(b.valid_from).bind(b.valid_to).bind(b.document_id).fetch_one(&mut *tx).await?;
    let pid: Uuid = v["id"]
        .as_str()
        .and_then(|s| s.parse().ok())
        .unwrap_or_default();
    audit_user(
        &mut *tx,
        u.id,
        "org.permit.create",
        "org_permits",
        Some(pid),
        json!({"org_id": id}),
    )
    .await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(v)))
}

/// 把仍綁著失效許可（未核准 / 已過期）、未關閉的據點一次換成這張有效許可；不改 site_status，恢復營運由使用者自己按。
async fn apply_permit(
    u: CurrentUser,
    State(st): State<AppState>,
    Path((id, pid)): Path<(Uuid, Uuid)>,
) -> R<Json<Value>> {
    let mut tx = crate::begin_contended(&st.pool).await?;
    lock(&mut tx, id).await?; // 許可審核也先鎖組織列，所以交易內讀到的許可狀態不會被並行撤銷；之後才動 wishlists
    require_role(&mut *tx, id, u.id, "coordinator").await?;
    let valid: bool = sqlx::query_scalar(
        "SELECT status = 'approved' AND (now() AT TIME ZONE 'Asia/Taipei')::date BETWEEN valid_from AND valid_to
           FROM org_permits WHERE id=$1 AND org_id=$2")
        .bind(pid).bind(id).fetch_optional(&mut *tx).await?.ok_or(AppError::NotFound)?;
    if !valid {
        return Err(AppError::problem(
            409,
            "PERMIT_INVALID",
            "許可未核准或已過期",
        ));
    }
    let ids: Vec<Uuid> = sqlx::query_scalar(
        "UPDATE wishlists w SET permit_id=$2 FROM org_permits old
          WHERE old.id = w.permit_id AND w.org_id=$1 AND w.type='relief' AND w.status <> 'closed' AND w.deleted_at IS NULL
            AND w.permit_id <> $2
            AND (old.status <> 'approved' OR (now() AT TIME ZONE 'Asia/Taipei')::date > old.valid_to)
          RETURNING w.id")
        .bind(id).bind(pid).fetch_all(&mut *tx).await?;
    if !ids.is_empty() {
        audit_user(
            &mut *tx,
            u.id,
            "org.permit.apply",
            "org_permits",
            Some(pid),
            json!({"org_id": id, "wishlist_ids": ids}),
        )
        .await?;
    }
    tx.commit().await?;
    Ok(Json(json!({"updated": ids.len()})))
}
