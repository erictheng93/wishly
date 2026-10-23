//! 品牌館與精選商品目錄：公開 GET /public/brands、/public/catalog/products（不讀 cookie，可被 CDN 快取），
//! 營運後台 /admin/brands、/admin/catalog/products（`Staff`，寫 audit_logs）。無 DELETE：以 active=false 下架，ref 必須穩定。
//! brands 是展示用品牌，不是 docs/04 的 merchants；catalog_products 與 P2 同一張表（見 0028）。
//! shortcut: 公開回應只 max-age=60 且不 purge CDN，改品牌 / 商品最多一分鐘後生效；要即時再接 cdn::purge。
use crate::{
    admin::{audit, Staff},
    error::AppError,
    public::image_url,
    validate, AppState,
};
use axum::{
    extract::{Path, Query, State},
    http::header,
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/public/catalog/products", get(public_products))
        .route("/public/brands", get(public_brands))
        .route("/public/brands/{slug}", get(public_brand))
        .route("/admin/brands", get(admin_brands).post(create_brand))
        .route("/admin/brands/{id}", axum::routing::patch(patch_brand))
        .route(
            "/admin/catalog/products",
            get(admin_products).post(create_product),
        )
        .route(
            "/admin/catalog/products/{id}",
            axum::routing::patch(patch_product),
        )
}

const PAGE: i64 = 24;
/// 公開目錄最多翻到第幾頁（超過的頁碼會被夾回，所以最後一頁不給 next_page）
const MAX_PAGE: i64 = 100;
const RECIPIENTS: &[&str] = &["partner", "elder", "baby", "friend"];
const OCCASIONS: &[&str] = &["birthday", "full_month", "housewarming", "holiday"];
type R<T> = Result<T, AppError>;

fn cached(body: Value) -> Response {
    ([(header::CACHE_CONTROL, "public, max-age=60")], Json(body)).into_response()
}

// ---------- 公開 ----------
#[derive(Deserialize)]
struct ProductQ {
    brand: Option<String>,
    recipient: Option<String>,
    occasion: Option<String>,
    budget: Option<String>,
    page: Option<i64>,
}

fn enum_opt<'a>(v: &'a Option<String>, ok: &[&str], ptr: &str) -> R<Option<&'a str>> {
    match v.as_deref() {
        None | Some("") => Ok(None),
        Some(s) if ok.contains(&s) => Ok(Some(s)),
        _ => Err(AppError::invalid(
            ptr,
            "ENUM",
            &format!("必須是 {}", ok.join(" / ")),
        )),
    }
}

type PubRow = (
    String,
    String,
    Option<String>,
    Option<String>,
    i64,
    Option<String>,
    String,
    String,
    Vec<String>,
    Vec<String>,
    Option<String>,
    Option<String>,
);

async fn public_products(State(st): State<AppState>, Query(q): Query<ProductQ>) -> R<Response> {
    let page = q.page.unwrap_or(1).clamp(1, MAX_PAGE);
    let rec = enum_opt(&q.recipient, RECIPIENTS, "/recipient")?;
    let occ = enum_opt(&q.occasion, OCCASIONS, "/occasion")?;
    // 價格區間（含邊界）：lt1000 <1000、1000-3000 含兩端、gt3000 >3000
    let (lo, hi): (Option<i64>, Option<i64>) =
        match enum_opt(&q.budget, &["lt1000", "1000-3000", "gt3000"], "/budget")? {
            Some("lt1000") => (None, Some(999)),
            Some("1000-3000") => (Some(1000), Some(3000)),
            Some(_) => (Some(3001), None),
            None => (None, None),
        };
    let brand = q.brand.as_deref().filter(|s| !s.is_empty());
    let rows: Vec<PubRow> = sqlx::query_as(
        "SELECT p.ref, p.title, p.spec, p.note, p.price_amount, p.image_key, p.image_status::text, p.product_url,
                p.recipients, p.occasions, b.slug, b.name
         FROM catalog_products p LEFT JOIN brands b ON b.id = p.brand_id
         WHERE p.active AND (b.id IS NULL OR b.active)
           AND ($1::text IS NULL OR b.slug = $1)
           AND ($2::text IS NULL OR $2 = ANY(p.recipients))
           AND ($3::text IS NULL OR $3 = ANY(p.occasions))
           AND ($4::bigint IS NULL OR p.price_amount >= $4)
           AND ($5::bigint IS NULL OR p.price_amount <= $5)
         ORDER BY p.sort_order, p.created_at DESC, p.id LIMIT $6 OFFSET $7",
    )
    .bind(brand)
    .bind(rec)
    .bind(occ)
    .bind(lo)
    .bind(hi)
    .bind(PAGE + 1)
    .bind((page - 1) * PAGE)
    .fetch_all(&st.pool)
    .await?;
    let more = rows.len() as i64 > PAGE;
    let products: Vec<Value> = rows
        .into_iter()
        .take(PAGE as usize)
        .map(|(r, title, spec, note, price, ik, is, url, rc, oc, bs, bn)| {
            json!({ "ref": r, "title": title, "spec": spec, "note": note, "price_amount": price,
                "image_url": image_url(ik.as_deref(), &is), "product_url": url, "recipients": rc, "occasions": oc,
                "brand": bs.map(|s| json!({ "slug": s, "name": bn })) })
        })
        .collect();
    Ok(cached(
        json!({ "products": products, "next_page": (more && page < MAX_PAGE).then_some(page + 1) }),
    ))
}

#[derive(FromRow)]
struct Brand {
    id: Uuid,
    slug: String,
    name: String,
    description: Option<String>,
    logo_key: Option<String>,
    logo_status: String,
    website_url: Option<String>,
    active: bool,
    sort_order: i32,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}
const BCOLS: &str = "id, slug, name, description, logo_key, logo_status::text AS logo_status, website_url, active, sort_order, created_at, updated_at";

impl Brand {
    fn public(&self, full: bool) -> Value {
        let mut v = json!({ "slug": self.slug, "name": self.name, "description": self.description,
            "logo_url": image_url(self.logo_key.as_deref(), &self.logo_status) });
        if full {
            v["website_url"] = json!(self.website_url);
        }
        v
    }
    fn admin(&self) -> Value {
        let mut v = self.public(true);
        v["id"] = json!(self.id);
        v["logo_status"] = json!(self.logo_status);
        v["active"] = json!(self.active);
        v["sort_order"] = json!(self.sort_order);
        v["created_at"] = json!(self.created_at);
        v["updated_at"] = json!(self.updated_at);
        v
    }
}

async fn public_brands(State(st): State<AppState>) -> R<Response> {
    let rows: Vec<Brand> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {BCOLS} FROM brands WHERE active ORDER BY sort_order, name, id"
    )))
    .fetch_all(&st.pool)
    .await?;
    Ok(cached(
        json!({ "brands": rows.iter().map(|b| b.public(false)).collect::<Vec<_>>() }),
    ))
}

async fn public_brand(State(st): State<AppState>, Path(slug): Path<String>) -> R<Response> {
    let b: Brand = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {BCOLS} FROM brands WHERE active AND slug = $1"
    )))
    .bind(slug)
    .fetch_optional(&st.pool)
    .await?
    .ok_or(AppError::NotFound)?;
    Ok(cached(json!({ "brand": b.public(true) })))
}

// ---------- 後台欄位驗證 ----------
/// 文字欄位：缺 → None（不動）；null / 空白 → Some(None)（必填欄位則 422）；超長 / 非字串 → 422
fn text(m: &Map<String, Value>, k: &str, max: usize, req: bool) -> R<Option<Option<String>>> {
    let ptr = format!("/{k}");
    let s = match m.get(k) {
        None => return Ok(None),
        Some(Value::Null) => String::new(),
        Some(Value::String(s)) => validate::text(s, &ptr, false)?,
        _ => return Err(AppError::invalid(&ptr, "TYPE", "必須是字串")),
    };
    if s.is_empty() && req {
        return Err(AppError::invalid(&ptr, "REQUIRED", "必填"));
    }
    if s.chars().count() > max {
        return Err(AppError::invalid(
            &ptr,
            "TOO_LONG",
            &format!("最多 {max} 字"),
        ));
    }
    Ok(Some((!s.is_empty()).then_some(s)))
}

fn http_url(m: &Map<String, Value>, k: &str, req: bool) -> R<Option<Option<String>>> {
    let v = text(m, k, 2000, req)?;
    if let Some(Some(u)) = &v {
        if !(u.starts_with("http://") || u.starts_with("https://"))
            || u.contains(char::is_whitespace)
        {
            return Err(AppError::invalid(
                &format!("/{k}"),
                "FORMAT",
                "須為 http(s) 網址",
            ));
        }
    }
    Ok(v)
}

/// 圖片 key：須為 uploads（purpose=catalog）發出的格式
fn image_key(m: &Map<String, Value>, k: &str) -> R<Option<Option<String>>> {
    let v = text(m, k, 200, false)?;
    if let Some(Some(s)) = &v {
        if !(crate::uploads::valid_key(s) && s.starts_with("catalog/")) {
            return Err(AppError::invalid(
                &format!("/{k}"),
                "FORMAT",
                "圖片 key 格式不正確",
            ));
        }
    }
    Ok(v)
}

fn flag(m: &Map<String, Value>, k: &str) -> R<Option<bool>> {
    match m.get(k) {
        None => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        _ => Err(AppError::invalid(&format!("/{k}"), "TYPE", "必須是布林值")),
    }
}

fn int(m: &Map<String, Value>, k: &str, lo: i64, hi: i64) -> R<Option<i64>> {
    match m.get(k) {
        None => Ok(None),
        Some(v) => v
            .as_i64()
            .filter(|n| (lo..=hi).contains(n))
            .map(Some)
            .ok_or_else(|| {
                AppError::invalid(
                    &format!("/{k}"),
                    "RANGE",
                    &format!("須為 {lo}..{hi} 的整數"),
                )
            }),
    }
}

fn vocab(m: &Map<String, Value>, k: &str, ok: &[&str]) -> R<Option<Vec<String>>> {
    let Some(v) = m.get(k) else { return Ok(None) };
    let bad = || {
        AppError::invalid(
            &format!("/{k}"),
            "ENUM",
            &format!("元素須為 {}", ok.join(" / ")),
        )
    };
    let mut out: Vec<String> = Vec::new();
    for x in v.as_array().ok_or_else(bad)? {
        let s = x.as_str().filter(|s| ok.contains(s)).ok_or_else(bad)?;
        if !out.iter().any(|o| o == s) {
            out.push(s.to_string());
        }
    }
    Ok(Some(out))
}

fn body(v: Value) -> R<Map<String, Value>> {
    match v {
        Value::Object(m) => Ok(m),
        _ => Err(AppError::invalid("", "TYPE", "必須是 JSON 物件")),
    }
}

/// PATCH 不可改的欄位
fn immutable(m: &Map<String, Value>, k: &str) -> R<()> {
    if m.contains_key(k) {
        return Err(AppError::invalid(
            &format!("/{k}"),
            "IMMUTABLE",
            "建立後不可修改",
        ));
    }
    Ok(())
}

fn conflict(e: sqlx::Error, code: &'static str, what: &str) -> AppError {
    if e.as_database_error().and_then(|d| d.code()).as_deref() == Some("23505") {
        AppError::problem(409, code, format!("{what}已被使用"))
    } else {
        e.into()
    }
}

// ---------- 後台：品牌 ----------
impl Brand {
    fn merge(&mut self, m: &Map<String, Value>) -> R<()> {
        if let Some(Some(n)) = text(m, "name", 80, true)? {
            self.name = n;
        }
        if let Some(d) = text(m, "description", 500, false)? {
            self.description = d;
        }
        if let Some(u) = http_url(m, "website_url", false)? {
            self.website_url = u;
        }
        if let Some(k) = image_key(m, "logo_key")? {
            self.logo_key = k;
        }
        if let Some(a) = flag(m, "active")? {
            self.active = a;
        }
        if let Some(n) = int(m, "sort_order", -1_000_000, 1_000_000)? {
            self.sort_order = n as i32;
        }
        Ok(())
    }
}

async fn admin_brands(State(st): State<AppState>, _s: Staff) -> R<Response> {
    let rows: Vec<Brand> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {BCOLS} FROM brands ORDER BY sort_order, name, id"
    )))
    .fetch_all(&st.pool)
    .await?;
    Ok(no_store(
        json!({ "brands": rows.iter().map(Brand::admin).collect::<Vec<_>>() }),
        200,
    ))
}

fn no_store(v: Value, status: u16) -> Response {
    (
        axum::http::StatusCode::from_u16(status).unwrap(),
        [(header::CACHE_CONTROL, "private, no-store")],
        Json(v),
    )
        .into_response()
}

async fn create_brand(
    State(st): State<AppState>,
    Staff(sid): Staff,
    Json(v): Json<Value>,
) -> R<Response> {
    let m = body(v)?;
    let slug = text(&m, "slug", 40, true)?
        .flatten()
        .ok_or_else(|| AppError::invalid("/slug", "REQUIRED", "必填"))?;
    let ok = (2..=40).contains(&slug.len())
        && slug
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if !ok {
        return Err(AppError::invalid(
            "/slug",
            "FORMAT",
            "slug 須為 2-40 碼小寫英數與 -",
        ));
    }
    let mut b = Brand {
        id: Uuid::nil(),
        slug,
        name: String::new(),
        description: None,
        logo_key: None,
        logo_status: "none".into(),
        website_url: None,
        active: false,
        sort_order: 0,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    b.merge(&m)?;
    if b.name.is_empty() {
        return Err(AppError::invalid("/name", "REQUIRED", "必填"));
    }
    let mut tx = st.pool.begin().await?;
    let row: Brand = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "INSERT INTO brands (slug, name, description, logo_key, logo_status, website_url, active, sort_order)
         VALUES ($1,$2,$3,$4, CASE WHEN $4::text IS NULL THEN 'none' ELSE 'ready' END::image_status,$5,$6,$7) RETURNING {BCOLS}")))
        .bind(&b.slug).bind(&b.name).bind(&b.description).bind(&b.logo_key).bind(&b.website_url).bind(b.active).bind(b.sort_order)
        .fetch_one(&mut *tx).await.map_err(|e| conflict(e, "SLUG_TAKEN", "slug"))?;
    audit(
        &mut *tx,
        sid,
        "brand.create",
        "brands",
        Some(row.id),
        json!({ "after": m }),
    )
    .await?;
    tx.commit().await?;
    Ok(no_store(json!({ "brand": row.admin() }), 201))
}

async fn patch_brand(
    State(st): State<AppState>,
    Staff(sid): Staff,
    Path(id): Path<Uuid>,
    Json(v): Json<Value>,
) -> R<Response> {
    let m = body(v)?;
    immutable(&m, "slug")?;
    let mut tx = st.pool.begin().await?;
    let mut b: Brand = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {BCOLS} FROM brands WHERE id=$1 FOR UPDATE"
    )))
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(AppError::NotFound)?;
    b.merge(&m)?;
    let row: Brand = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "UPDATE brands SET name=$2, description=$3, logo_key=$4, logo_status=CASE WHEN $4::text IS NULL THEN 'none' ELSE 'ready' END::image_status,
                website_url=$5, active=$6, sort_order=$7 WHERE id=$1 RETURNING {BCOLS}")))
        .bind(id).bind(&b.name).bind(&b.description).bind(&b.logo_key).bind(&b.website_url).bind(b.active).bind(b.sort_order)
        .fetch_one(&mut *tx).await?;
    audit(
        &mut *tx,
        sid,
        "brand.update",
        "brands",
        Some(id),
        json!({ "patch": m }),
    )
    .await?;
    tx.commit().await?;
    Ok(no_store(json!({ "brand": row.admin() }), 200))
}

// ---------- 後台：商品 ----------
#[derive(FromRow)]
struct Prod {
    id: Uuid,
    #[sqlx(rename = "ref")]
    ref_: String,
    brand_id: Option<Uuid>,
    title: String,
    spec: Option<String>,
    note: Option<String>,
    price_amount: i64,
    image_key: Option<String>,
    image_status: String,
    product_url: String,
    recipients: Vec<String>,
    occasions: Vec<String>,
    active: bool,
    sort_order: i32,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}
const PCOLS: &str = "id, ref, brand_id, title, spec, note, price_amount, image_key, image_status::text AS image_status, product_url, recipients, occasions, active, sort_order, created_at, updated_at";

impl Prod {
    fn admin(&self) -> Value {
        json!({ "id": self.id, "ref": self.ref_, "brand_id": self.brand_id, "title": self.title, "spec": self.spec, "note": self.note,
            "price_amount": self.price_amount, "image_url": image_url(self.image_key.as_deref(), &self.image_status),
            "image_key": self.image_key, "image_status": self.image_status, "product_url": self.product_url,
            "recipients": self.recipients, "occasions": self.occasions, "active": self.active, "sort_order": self.sort_order,
            "created_at": self.created_at, "updated_at": self.updated_at })
    }
    async fn merge(&mut self, m: &Map<String, Value>, pool: &PgPool) -> R<()> {
        if let Some(Some(t)) = text(m, "title", 120, true)? {
            self.title = t;
        }
        if let Some(s) = text(m, "spec", 200, false)? {
            self.spec = s;
        }
        if let Some(n) = text(m, "note", 300, false)? {
            self.note = n;
        }
        if let Some(n) = int(m, "price_amount", 0, 100_000_000)? {
            self.price_amount = n;
        }
        if let Some(Some(u)) = http_url(m, "product_url", true)? {
            self.product_url = u;
        }
        if let Some(k) = image_key(m, "image_key")? {
            self.image_key = k;
        }
        if let Some(r) = vocab(m, "recipients", RECIPIENTS)? {
            self.recipients = r;
        }
        if let Some(o) = vocab(m, "occasions", OCCASIONS)? {
            self.occasions = o;
        }
        if let Some(a) = flag(m, "active")? {
            self.active = a;
        }
        if let Some(n) = int(m, "sort_order", -1_000_000, 1_000_000)? {
            self.sort_order = n as i32;
        }
        match m.get("brand_id") {
            None => {}
            Some(Value::Null) => self.brand_id = None,
            Some(v) => {
                let id = v
                    .as_str()
                    .and_then(|s| Uuid::parse_str(s).ok())
                    .ok_or_else(|| AppError::invalid("/brand_id", "FORMAT", "須為 UUID 或 null"))?;
                let ok: bool =
                    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM brands WHERE id=$1)")
                        .bind(id)
                        .fetch_one(pool)
                        .await?;
                if !ok {
                    return Err(AppError::invalid("/brand_id", "NOT_FOUND", "品牌不存在"));
                }
                self.brand_id = Some(id);
            }
        }
        Ok(())
    }
}

#[derive(Deserialize)]
struct AdminProductQ {
    brand_id: Option<Uuid>,
    page: Option<i64>,
}

async fn admin_products(
    State(st): State<AppState>,
    _s: Staff,
    Query(q): Query<AdminProductQ>,
) -> R<Response> {
    let page = q.page.unwrap_or(1).clamp(1, 1000);
    let mut rows: Vec<Prod> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {PCOLS} FROM catalog_products WHERE ($1::uuid IS NULL OR brand_id = $1)
         ORDER BY sort_order, created_at DESC, id LIMIT $2 OFFSET $3"
    )))
    .bind(q.brand_id)
    .bind(PAGE + 1)
    .bind((page - 1) * PAGE)
    .fetch_all(&st.pool)
    .await?;
    let more = rows.len() as i64 > PAGE;
    rows.truncate(PAGE as usize);
    Ok(no_store(
        json!({ "products": rows.iter().map(Prod::admin).collect::<Vec<_>>(), "next_page": more.then_some(page + 1) }),
        200,
    ))
}

async fn create_product(
    State(st): State<AppState>,
    Staff(sid): Staff,
    Json(v): Json<Value>,
) -> R<Response> {
    let m = body(v)?;
    let r = text(&m, "ref", 64, true)?
        .flatten()
        .ok_or_else(|| AppError::invalid("/ref", "REQUIRED", "必填"))?;
    if !crate::inspire::valid_ref(&r) {
        return Err(AppError::invalid(
            "/ref",
            "FORMAT",
            "ref 須為 1-64 碼英數、_、-",
        ));
    }
    let mut p = Prod {
        id: Uuid::nil(),
        ref_: r,
        brand_id: None,
        title: String::new(),
        spec: None,
        note: None,
        price_amount: -1,
        image_key: None,
        image_status: "none".into(),
        product_url: String::new(),
        recipients: vec![],
        occasions: vec![],
        active: false,
        sort_order: 0,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    p.merge(&m, &st.pool).await?;
    for (bad, k) in [
        (p.title.is_empty(), "title"),
        (p.price_amount < 0, "price_amount"),
        (p.product_url.is_empty(), "product_url"),
    ] {
        if bad {
            return Err(AppError::invalid(&format!("/{k}"), "REQUIRED", "必填"));
        }
    }
    let mut tx = st.pool.begin().await?;
    let row: Prod = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "INSERT INTO catalog_products (ref, brand_id, title, spec, note, price_amount, image_key, image_status, product_url, recipients, occasions, active, sort_order)
         VALUES ($1,$2,$3,$4,$5,$6,$7, CASE WHEN $7::text IS NULL THEN 'none' ELSE 'ready' END::image_status,$8,$9,$10,$11,$12) RETURNING {PCOLS}")))
        .bind(&p.ref_).bind(p.brand_id).bind(&p.title).bind(&p.spec).bind(&p.note).bind(p.price_amount).bind(&p.image_key)
        .bind(&p.product_url).bind(&p.recipients).bind(&p.occasions).bind(p.active).bind(p.sort_order)
        .fetch_one(&mut *tx).await.map_err(|e| conflict(e, "REF_TAKEN", "ref"))?;
    audit(
        &mut *tx,
        sid,
        "catalog_product.create",
        "catalog_products",
        Some(row.id),
        json!({ "after": m }),
    )
    .await?;
    tx.commit().await?;
    Ok(no_store(json!({ "product": row.admin() }), 201))
}

async fn patch_product(
    State(st): State<AppState>,
    Staff(sid): Staff,
    Path(id): Path<Uuid>,
    Json(v): Json<Value>,
) -> R<Response> {
    let m = body(v)?;
    immutable(&m, "ref")?;
    let mut tx = st.pool.begin().await?;
    let mut p: Prod = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {PCOLS} FROM catalog_products WHERE id=$1 FOR UPDATE"
    )))
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(AppError::NotFound)?;
    p.merge(&m, &st.pool).await?;
    let row: Prod = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "UPDATE catalog_products SET brand_id=$2, title=$3, spec=$4, note=$5, price_amount=$6, image_key=$7,
                image_status=CASE WHEN $7::text IS NULL THEN 'none' ELSE 'ready' END::image_status,
                product_url=$8, recipients=$9, occasions=$10, active=$11, sort_order=$12 WHERE id=$1 RETURNING {PCOLS}")))
        .bind(id).bind(p.brand_id).bind(&p.title).bind(&p.spec).bind(&p.note).bind(p.price_amount).bind(&p.image_key)
        .bind(&p.product_url).bind(&p.recipients).bind(&p.occasions).bind(p.active).bind(p.sort_order)
        .fetch_one(&mut *tx).await?;
    audit(
        &mut *tx,
        sid,
        "catalog_product.update",
        "catalog_products",
        Some(id),
        json!({ "patch": m }),
    )
    .await?;
    tx.commit().await?;
    Ok(no_store(json!({ "product": row.admin() }), 200))
}
