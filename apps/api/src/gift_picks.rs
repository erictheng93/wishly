//! GET /users/{handle}/gift-picks：依對方「看得到的」興趣推薦精選商品。見 spec 2026-11-06-interest-gift-picks。
use crate::catalog::{enum_opt, product_json, PubRow, OCCASIONS};
use crate::profiles::{birthday_visible, can_see, relation_of, Relation};
use crate::{access, error::AppError, AppState};
use axum::{
    extract::{Path, Query, State},
    http::{header, request::Parts},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

pub fn routes() -> Router<AppState> {
    Router::new().route("/users/{handle}/gift-picks", get(gift_picks))
}

#[derive(Deserialize)]
struct Q {
    occasion: Option<String>,
    limit: Option<i64>,
}

fn no_store(body: Value) -> Response {
    ([(header::CACHE_CONTROL, "no-store")], Json(body)).into_response()
}

async fn gift_picks(
    State(st): State<AppState>,
    Path(handle): Path<String>,
    Query(q): Query<Q>,
    parts: Parts,
) -> Result<Response, AppError> {
    let occ = enum_opt(&q.occasion, OCCASIONS, "/occasion")?;
    let limit = q.limit.unwrap_or(6);
    if !(1..=50).contains(&limit) {
        return Err(AppError::invalid("/limit", "RANGE", "必須介於 1 到 50"));
    }
    let limit = limit as usize;
    let viewer = access::viewer(&parts, &st).await?;
    // handle 規則同 profiles::profile：user id 只給登入者用
    let by_id = Uuid::try_parse(&handle).ok();
    if by_id.is_some() && viewer.is_none() {
        return Err(AppError::NotFound);
    }
    #[allow(clippy::type_complexity)]
    let (uid, ints, iv, dis, gv, birthday, bv): (Uuid, Vec<String>, String, Vec<String>, String, Option<String>, String) = sqlx::query_as(
        "SELECT id, interests, interests_vis::text, gift_dislikes, gift_prefs_vis::text, birthday, birthday_visibility::text FROM users
          WHERE (id = $1 OR ($1 IS NULL AND handle = $2)) AND deleted_at IS NULL AND blocked_at IS NULL",
    )
    .bind(by_id)
    .bind(handle.to_lowercase())
    .fetch_optional(&st.pool)
    .await?
    .ok_or(AppError::NotFound)?;
    let Relation {
        rel,
        blocked_by_owner,
        i_blocked,
        friend,
    } = relation_of(&st.pool, uid, viewer).await?;
    if blocked_by_owner {
        return Err(AppError::NotFound);
    }
    if i_blocked {
        return Ok(no_store(
            json!({ "basis": "fallback", "products": [], "excluded": [] }),
        ));
    }
    let ints = if can_see(&iv, rel) { ints } else { vec![] };
    // 去頭尾空白、小寫；長度（字元數）< 2 不比對，避免單字誤殺
    let dis: Vec<String> = if can_see(&gv, rel) {
        dis.iter()
            .map(|d| d.trim().to_lowercase())
            .filter(|d| d.chars().count() >= 2)
            .collect()
    } else {
        vec![]
    };
    let is_self = viewer == Some(uid);
    let near_birthday = birthday
        .as_deref()
        .filter(|_| birthday_visible(&bv, is_self, friend))
        .and_then(|b| {
            let today = (chrono::Utc::now() + chrono::Duration::hours(8)).date_naive();
            crate::friends::days_until(today, b)
        })
        .is_some_and(|d| d <= 30);

    let rows: Vec<PubRow> = sqlx::query_as(
        "SELECT p.ref, p.title, p.spec, p.note, p.price_amount, p.image_key, p.image_status::text, p.product_url,
                p.recipients, p.occasions, p.interests, b.slug, b.name
         FROM catalog_products p LEFT JOIN brands b ON b.id = p.brand_id
         WHERE p.active AND (b.id IS NULL OR b.active) AND ($1::text IS NULL OR $1 = ANY(p.occasions))
         ORDER BY p.sort_order, p.created_at DESC, p.id",
    )
    .bind(occ)
    .fetch_all(&st.pool)
    .await?;
    // 排除不想收到（標題或品牌名）
    let (rows, gone): (Vec<PubRow>, Vec<PubRow>) = rows.into_iter().partition(|r| {
        let text = format!("{}\n{}", r.1, r.12.as_deref().unwrap_or("")).to_lowercase();
        !dis.iter().any(|d| text.contains(d.as_str()))
    });
    let excluded: Vec<&str> = gone.iter().map(|r| r.0.as_str()).collect();
    let matched =
        |r: &PubRow| -> Vec<String> { ints.iter().filter(|i| r.10.contains(i)).cloned().collect() };
    let mut scored: Vec<(Vec<String>, PubRow)> = rows
        .iter()
        .map(|r| (matched(r), r.clone()))
        .filter(|(m, _)| !m.is_empty())
        .collect();
    // 穩定排序：同分維持 SQL 的 sort_order / id 順序
    scored.sort_by_key(|(m, _)| std::cmp::Reverse(m.len()));
    let (basis, picks): (&str, Vec<(Vec<String>, PubRow)>) = if scored.is_empty() {
        let want_birthday = near_birthday && occ.is_none();
        let fb = rows
            .into_iter()
            .filter(|r| {
                if want_birthday {
                    r.9.iter().any(|o| o == "birthday")
                } else {
                    r.8.iter().any(|o| o == "friend")
                }
            })
            .map(|r| (vec![], r))
            .collect();
        ("fallback", fb)
    } else {
        ("interests", scored)
    };
    let products: Vec<Value> = picks
        .into_iter()
        .take(limit)
        .map(|(m, r)| {
            let mut v = product_json(r);
            v["matched"] = json!(m);
            v
        })
        .collect();
    Ok(no_store(
        json!({ "basis": basis, "products": products, "excluded": excluded }),
    ))
}
