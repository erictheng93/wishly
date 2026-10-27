//! 災害事件：建立、查重、編輯、合併與下架（spec 2026-10-27-org-relief-registration-design）。
//! 鎖序：relief_events（依 id 由小到大）→ wishlists。
use crate::{
    admin::{audit, Staff},
    error::AppError,
    orgs::{audit_user, member_role, require_role},
    session::CurrentUser,
    validate,
    wishlists::new_slug,
    AppState,
};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, patch, post},
    Json, Router,
};
use chrono::NaiveDate;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

type R<T> = Result<T, AppError>;

/// 台灣 22 縣市正式名稱（前端同一份清單，順序固定）
pub const COUNTIES: [&str; 22] = [
    "臺北市", "新北市", "桃園市", "臺中市", "臺南市", "高雄市", "基隆市", "新竹市", "嘉義市", "新竹縣", "苗栗縣", "彰化縣",
    "南投縣", "雲林縣", "嘉義縣", "屏東縣", "宜蘭縣", "花蓮縣", "臺東縣", "澎湖縣", "金門縣", "連江縣",
];

const HAZARDS: [&str; 5] = ["earthquake", "typhoon", "flood", "fire", "other"];

/// 事件 JSON；欄位固定（前端依此）
const J: &str = "jsonb_build_object('id',id,'slug',slug,'name',name,'hazard_type',hazard_type,'counties',counties,'area_note',area_note,
    'occurred_on',occurred_on,'description',description,'created_by_org',created_by_org,'merged_into',merged_into,
    'moderation_status',moderation_status,'created_at',created_at)";

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/orgs/{id}/events", post(create).get(list))
        .route("/relief-events/similar", get(similar))
        .route("/relief-events/{id}", patch(edit))
        .route("/admin/relief-events/{id}/merge", post(merge))
        .route("/admin/relief-events/{id}", patch(moderate))
}

fn check_county(c: &str, pointer: &str) -> R<()> {
    if COUNTIES.contains(&c) { Ok(()) } else { Err(AppError::invalid(pointer, "ENUM", "縣市名稱不正確")) }
}

fn opt_text(v: &Option<String>, pointer: &str, max: usize) -> R<Option<String>> {
    let Some(s) = v else { return Ok(None) };
    let t = validate::text(s, pointer, true)?;
    if t.chars().count() > max {
        return Err(AppError::invalid(pointer, "LENGTH", "長度超過限制"));
    }
    Ok(Some(t))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewEvent {
    name: String,
    hazard_type: String,
    counties: Vec<String>,
    area_note: Option<String>,
    occurred_on: NaiveDate,
    description: Option<String>,
}

async fn create(u: CurrentUser, State(st): State<AppState>, Path(org): Path<Uuid>, Json(b): Json<NewEvent>) -> R<(StatusCode, Json<Value>)> {
    require_role(&st.pool, org, u.id, "coordinator").await?;
    let verified: bool = sqlx::query_scalar("SELECT verification_status = 'verified' FROM organizations WHERE id=$1").bind(org).fetch_one(&st.pool).await?;
    if !verified {
        return Err(AppError::problem(403, "ORG_NOT_VERIFIED", "組織通過審核後才能建立災害事件"));
    }
    let name = validate::text(&b.name, "/name", false)?;
    if name.is_empty() || name.chars().count() > 80 {
        return Err(AppError::invalid("/name", "LENGTH", "名稱須為 1 至 80 字"));
    }
    if !HAZARDS.contains(&b.hazard_type.as_str()) {
        return Err(AppError::invalid("/hazard_type", "ENUM", "災害類型不正確"));
    }
    if b.counties.is_empty() || b.counties.len() > COUNTIES.len() {
        return Err(AppError::invalid("/counties", "LENGTH", "至少選一個縣市"));
    }
    for c in &b.counties {
        check_county(c, "/counties")?;
    }
    let area = opt_text(&b.area_note, "/area_note", 200)?;
    let desc = opt_text(&b.description, "/description", 2000)?;
    let mut tx = st.pool.begin().await?;
    let v: Value = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "WITH e AS (INSERT INTO relief_events (slug,name,hazard_type,counties,area_note,occurred_on,description,created_by_org)
           VALUES ($1,$2,$3,$4,$5,$6,$7,$8) RETURNING *) SELECT {J} FROM e")))
        .bind(new_slug()).bind(&name).bind(&b.hazard_type).bind(&b.counties).bind(&area).bind(b.occurred_on).bind(&desc).bind(org)
        .fetch_one(&mut *tx).await?;
    let id: Uuid = v["id"].as_str().and_then(|s| s.parse().ok()).ok_or(AppError::NotFound)?;
    audit_user(&mut *tx, u.id, "relief_event.create", "relief_event", Some(id), json!({"org_id": org})).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(v)))
}

/// 本組織建立的事件，或有本組織據點的事件；不含已被合併者
async fn list(u: CurrentUser, State(st): State<AppState>, Path(org): Path<Uuid>) -> R<Json<Vec<Value>>> {
    member_role(&st.pool, org, u.id).await?.ok_or(AppError::NotFound)?;
    Ok(Json(sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT {J} FROM relief_events WHERE merged_into IS NULL AND (created_by_org = $1
           OR EXISTS (SELECT 1 FROM wishlists w WHERE w.event_id = relief_events.id AND w.org_id = $1)) ORDER BY created_at DESC, id")))
        .bind(org)
        .fetch_all(&st.pool).await?))
}

#[derive(Deserialize)]
struct SimilarQ {
    county: String,
    date: NaiveDate,
}

async fn similar(_u: CurrentUser, State(st): State<AppState>, Query(q): Query<SimilarQ>) -> R<Json<Vec<Value>>> {
    check_county(&q.county, "/county")?;
    Ok(Json(sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT {J} FROM relief_events WHERE $1 = ANY(counties) AND occurred_on BETWEEN $2::date - 7 AND $2::date + 7
           AND merged_into IS NULL AND moderation_status = 'ok' ORDER BY occurred_on DESC, id LIMIT 10")))
        .bind(&q.county).bind(q.date).fetch_all(&st.pool).await?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Edit {
    name: Option<String>,
    description: Option<String>,
    area_note: Option<String>,
}

async fn edit(u: CurrentUser, State(st): State<AppState>, Path(id): Path<Uuid>, Json(b): Json<Edit>) -> R<Json<Value>> {
    let name = opt_text(&b.name, "/name", 80)?;
    if name.as_deref() == Some("") {
        return Err(AppError::invalid("/name", "LENGTH", "名稱不可為空"));
    }
    let desc = opt_text(&b.description, "/description", 2000)?;
    let area = opt_text(&b.area_note, "/area_note", 200)?;
    let mut tx = st.pool.begin().await?;
    let (owner_org, merged, mod_status): (Uuid, Option<Uuid>, String) =
        sqlx::query_as("SELECT created_by_org, merged_into, moderation_status::text FROM relief_events WHERE id=$1 FOR UPDATE")
            .bind(id).fetch_optional(&mut *tx).await?.ok_or(AppError::NotFound)?;
    // 非建立組織成員也是 403（可附掛據點，但不可改事件）
    match require_role(&mut *tx, owner_org, u.id, "coordinator").await {
        Err(AppError::NotFound) => return Err(AppError::problem(403, "ORG_ROLE_REQUIRED", "僅建立事件的組織可編輯")),
        r => r?,
    };
    if merged.is_some() || mod_status == "hidden" {
        return Err(AppError::problem(409, "EVENT_NOT_EDITABLE", "已合併或下架的事件不可編輯"));
    }
    let v: Value = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "WITH e AS (UPDATE relief_events SET name=COALESCE($2,name), description=COALESCE($3,description), area_note=COALESCE($4,area_note)
           WHERE id=$1 RETURNING *) SELECT {J} FROM e")))
        .bind(id).bind(&name).bind(&desc).bind(&area).fetch_one(&mut *tx).await?;
    audit_user(&mut *tx, u.id, "relief_event.edit", "relief_event", Some(id), json!({"org_id": owner_org})).await?;
    let slugs: Vec<String> = sqlx::query_scalar("SELECT slug FROM wishlists WHERE event_id=$1").bind(id).fetch_all(&mut *tx).await?;
    tx.commit().await?;
    crate::cdn::purge_wishlists(slugs);
    Ok(Json(v))
}

#[derive(Deserialize)]
struct MergeBody {
    into: Uuid,
}

async fn merge(Staff(staff): Staff, State(st): State<AppState>, Path(id): Path<Uuid>, Json(b): Json<MergeBody>) -> R<Json<Value>> {
    if b.into == id {
        return Err(AppError::invalid("/into", "SELF", "不可合併到自己"));
    }
    let mut tx = st.pool.begin().await?;
    // 兩列依 id 由小到大鎖，A→B 與 B→A 同時進來也不會互鎖
    let rows = sqlx::query_as::<_, (Uuid, Option<Uuid>, String)>("SELECT id, merged_into, moderation_status::text FROM relief_events WHERE id = ANY($1) ORDER BY id FOR UPDATE")
        .bind(vec![id, b.into]).fetch_all(&mut *tx).await?;
    if rows.len() != 2 {
        return Err(AppError::NotFound);
    }
    if rows.iter().any(|r| r.1.is_some()) {
        return Err(AppError::invalid("/into", "ALREADY_MERGED", "事件已被合併"));
    }
    if rows.iter().any(|r| r.0 == b.into && r.2 == "hidden") {
        return Err(AppError::invalid("/into", "EVENT_HIDDEN", "不可合併到已下架的事件"));
    }
    let slugs: Vec<String> = sqlx::query_scalar("UPDATE wishlists SET event_id=$2 WHERE event_id=$1 RETURNING slug").bind(id).bind(b.into).fetch_all(&mut *tx).await?;
    sqlx::query("UPDATE relief_events SET merged_into=$2 WHERE id=$1").bind(id).bind(b.into).execute(&mut *tx).await?;
    audit(&mut *tx, staff, "relief_event.merge", "relief_event", Some(id), json!({"into": b.into, "sites": slugs.len()})).await?;
    tx.commit().await?;
    crate::cdn::purge_wishlists(slugs);
    Ok(Json(json!({"id": id, "merged_into": b.into})))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Moderate {
    moderation_status: String,
    moderation_reason: Option<String>,
}

async fn moderate(Staff(staff): Staff, State(st): State<AppState>, Path(id): Path<Uuid>, Json(b): Json<Moderate>) -> R<Json<Value>> {
    let hide = match b.moderation_status.as_str() {
        "hidden" => true,
        "ok" => false,
        _ => return Err(AppError::invalid("/moderation_status", "ENUM", "狀態不正確")),
    };
    let reason = opt_text(&b.moderation_reason, "/moderation_reason", 500)?.filter(|s| !s.is_empty());
    if hide && reason.is_none() {
        return Err(AppError::invalid("/moderation_reason", "REQUIRED", "下架須填寫原因"));
    }
    let mut tx = st.pool.begin().await?;
    sqlx::query_scalar::<_, Uuid>("SELECT id FROM relief_events WHERE id=$1 FOR UPDATE").bind(id).fetch_optional(&mut *tx).await?.ok_or(AppError::NotFound)?;
    sqlx::query("UPDATE relief_events SET moderation_status=$2::moderation_status, moderation_reason=$3 WHERE id=$1")
        .bind(id).bind(&b.moderation_status).bind(if hide { reason.clone() } else { None }).execute(&mut *tx).await?;
    let slugs: Vec<String> = if hide {
        sqlx::query_scalar("UPDATE wishlists SET site_status='paused' WHERE event_id=$1 AND type='relief' AND status='active' RETURNING slug").bind(id).fetch_all(&mut *tx).await?
    } else {
        vec![]
    };
    audit(&mut *tx, staff, "relief_event.moderate", "relief_event", Some(id), json!({"status": b.moderation_status, "reason": reason, "paused": slugs.len()})).await?;
    tx.commit().await?;
    crate::cdn::purge_wishlists(slugs);
    Ok(Json(json!({"id": id, "moderation_status": b.moderation_status})))
}
