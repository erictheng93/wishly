//! 個人頁重要日子：本人 CRUD（/me/dates）、個人頁可見清單、7 天前站內通知排程 `date_tick_at`。
use crate::{error::AppError, session::CurrentUser, AppState};
use axum::{
    extract::{Path, State},
    routing::get,
    Json, Router,
};
use chrono::{Datelike, NaiveDate};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

const MAX_DATES: i64 = 20;
const NOTIFY_DAYS: i64 = 7;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/me/dates", get(list).post(create))
        .route(
            "/me/dates/{id}",
            axum::routing::patch(update).delete(remove),
        )
}

fn today_tpe() -> NaiveDate {
    (chrono::Utc::now() + chrono::Duration::hours(8)).date_naive()
}

/// 用閏年 2000 驗證，允許 02-29；格式須剛好 MM-DD（與 auth.rs 生日同規則）
fn valid_mmdd(b: &str) -> bool {
    b.len() == 5 && NaiveDate::parse_from_str(&format!("2000-{b}"), "%Y-%m-%d").is_ok()
}

/// 距離下次發生的天數；一次性日子須是今天以後（年份對得上）才算。非閏年的 02-29 以 02-28 計。
pub(crate) fn upcoming(
    today: NaiveDate,
    md: &str,
    year: Option<i16>,
    recurring: bool,
) -> Option<i64> {
    let d = crate::friends::days_until(today, md)?;
    (recurring || year == Some(crate::birthday::year_of(today, d))).then_some(d)
}

fn check(
    title: &str,
    md: &str,
    year: Option<i16>,
    recurring: bool,
    vis: &str,
) -> Result<(), AppError> {
    let n = title.chars().count();
    if !(1..=30).contains(&n) {
        return Err(AppError::invalid("/title", "LENGTH", "標題須為 1–30 字"));
    }
    if !valid_mmdd(md) {
        return Err(AppError::invalid(
            "/month_day",
            "FORMAT",
            "日期格式須為 MM-DD",
        ));
    }
    if year.is_some_and(|y| !(1900..=2100).contains(&y)) {
        return Err(AppError::invalid("/year", "RANGE", "年份須在 1900–2100"));
    }
    if !recurring && year.is_none() {
        return Err(AppError::invalid(
            "/year",
            "REQUIRED",
            "不重複的日子須填年份",
        ));
    }
    if !matches!(vis, "public" | "friends" | "close" | "private") {
        return Err(AppError::invalid("/visibility", "INVALID", "可見性不正確"));
    }
    Ok(())
}

async fn list(u: CurrentUser, State(st): State<AppState>) -> Result<Json<Value>, AppError> {
    let rows: Vec<(Uuid, String, String, Option<i16>, bool, String)> = sqlx::query_as(
        "SELECT id, title, month_day, year, recurring, visibility::text FROM user_dates WHERE user_id = $1 ORDER BY month_day, created_at")
        .bind(u.id).fetch_all(&st.pool).await?;
    let today = today_tpe();
    let dates: Vec<Value> = rows.into_iter().map(|(id, title, md, year, recurring, vis)| {
        let now = (today.year(), format!("{:02}-{:02}", today.month(), today.day()));
        let past = !recurring && year.is_some_and(|y| (i32::from(y), md.clone()) < now);
        json!({ "id": id, "title": title, "month_day": md, "year": year, "recurring": recurring, "visibility": vis, "past": past })
    }).collect();
    Ok(Json(json!({ "dates": dates })))
}

/// 個人頁用：觀看者看得到且未過的日子，依 days_until 排序
pub(crate) async fn visible(
    pool: &PgPool,
    uid: Uuid,
    rel: crate::profiles::Rel,
) -> Result<Vec<Value>, AppError> {
    let rows: Vec<(String, String, Option<i16>, bool, String)> = sqlx::query_as(
        "SELECT title, month_day, year, recurring, visibility::text FROM user_dates WHERE user_id = $1")
        .bind(uid).fetch_all(pool).await?;
    let today = today_tpe();
    let mut out: Vec<(i64, Value)> = rows
        .into_iter()
        .filter(|r| crate::profiles::can_see(&r.4, rel))
        .filter_map(|(title, md, year, recurring, _)| {
            let d = upcoming(today, &md, year, recurring)?;
            let mut v = json!({ "title": title, "month_day": md, "days_until": d });
            if !recurring {
                v["year"] = json!(year);
            }
            Some((d, v))
        })
        .collect();
    out.sort_by_key(|x| x.0);
    Ok(out.into_iter().map(|x| x.1).collect())
}

#[derive(Deserialize)]
struct NewDate {
    title: String,
    month_day: String,
    year: Option<i16>,
    #[serde(default = "yes")]
    recurring: bool,
    #[serde(default = "friends")]
    visibility: String,
}
fn yes() -> bool {
    true
}
fn friends() -> String {
    "friends".into()
}

async fn create(
    u: CurrentUser,
    State(st): State<AppState>,
    Json(b): Json<NewDate>,
) -> Result<Json<Value>, AppError> {
    let title = b.title.trim().to_string();
    check(&title, &b.month_day, b.year, b.recurring, &b.visibility)?;
    let mut tx = st.pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('user_dates:' || $1::text, 0))")
        .bind(u.id)
        .execute(&mut *tx)
        .await?;
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM user_dates WHERE user_id = $1")
        .bind(u.id)
        .fetch_one(&mut *tx)
        .await?;
    if n >= MAX_DATES {
        return Err(AppError::problem(422, "LIMIT", "最多 20 筆重要日子"));
    }
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO user_dates (user_id, title, month_day, year, recurring, visibility) VALUES ($1, $2, $3, $4, $5, $6::text::profile_vis) RETURNING id")
        .bind(u.id).bind(&title).bind(&b.month_day).bind(b.year).bind(b.recurring).bind(&b.visibility)
        .fetch_one(&mut *tx).await?;
    tx.commit().await?;
    Ok(Json(json!({ "id": id })))
}

#[derive(Deserialize)]
struct Patch {
    title: Option<String>,
    month_day: Option<String>,
    #[serde(default, deserialize_with = "double_opt")]
    year: Option<Option<i16>>,
    recurring: Option<bool>,
    visibility: Option<String>,
}
fn double_opt<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Option<i16>>, D::Error> {
    Option::<i16>::deserialize(d).map(Some)
}

async fn update(
    u: CurrentUser,
    State(st): State<AppState>,
    Path(id): Path<Uuid>,
    Json(p): Json<Patch>,
) -> Result<Json<Value>, AppError> {
    let mut tx = st.pool.begin().await?;
    let (title, md, year, recurring, vis): (String, String, Option<i16>, bool, String) = sqlx::query_as(
        "SELECT title, month_day, year, recurring, visibility::text FROM user_dates WHERE id = $1 AND user_id = $2 FOR UPDATE")
        .bind(id).bind(u.id).fetch_optional(&mut *tx).await?.ok_or(AppError::NotFound)?;
    let title = p.title.map_or(title, |t| t.trim().to_string());
    let (nmd, nyear, nrec, nvis) = (
        p.month_day.clone().unwrap_or(md.clone()),
        p.year.unwrap_or(year),
        p.recurring.unwrap_or(recurring),
        p.visibility.clone().unwrap_or(vis.clone()),
    );
    check(&title, &nmd, nyear, nrec, &nvis)?;
    sqlx::query("UPDATE user_dates SET title=$2, month_day=$3, year=$4, recurring=$5, visibility=$6::text::profile_vis WHERE id=$1")
        .bind(id).bind(&title).bind(&nmd).bind(nyear).bind(nrec).bind(&nvis).execute(&mut *tx).await?;
    if (&nmd, nyear, nrec, &nvis) != (&md, year, recurring, &vis) {
        // 日期或對象改了：今年起的已寄紀錄作廢，排隊中的舊通知取消，讓排程重新評估
        sqlx::query("DELETE FROM user_date_sent WHERE date_id = $1 AND year >= $2")
            .bind(id)
            .bind(today_tpe().year() as i16)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE notifications SET status='cancelled' WHERE status='pending' AND kind='profile.date_soon' AND payload->>'date_id' = $1::text")
            .bind(id).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(Json(json!({ "ok": true })))
}

async fn remove(
    u: CurrentUser,
    State(st): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    let n = sqlx::query("DELETE FROM user_dates WHERE id = $1 AND user_id = $2")
        .bind(id)
        .bind(u.id)
        .execute(&st.pool)
        .await?
        .rows_affected();
    if n == 0 {
        return Err(AppError::NotFound);
    }
    Ok(Json(json!({ "ok": true })))
}

/// 排入 7 天內到期日子的好友通知，回傳筆數。補跑邏輯同 birthday：窗口內且該年未寄就排一次；冪等靠 user_date_sent 主鍵。
pub async fn date_tick_at(pool: &PgPool, today: NaiveDate) -> Result<u64, sqlx::Error> {
    let mut queued = 0;
    let rows: Vec<(Uuid, Uuid, String, Option<i16>, bool, String)> = sqlx::query_as(
        "SELECT d.id, d.user_id, d.month_day, d.year, d.recurring, d.visibility::text
           FROM user_dates d JOIN users u ON u.id = d.user_id AND u.deleted_at IS NULL AND u.blocked_at IS NULL
          WHERE d.visibility <> 'private'")
        .fetch_all(pool).await?;
    for (id, owner, md, year, recurring, vis) in rows {
        let Some(d) = upcoming(today, &md, year, recurring).filter(|d| *d <= NOTIFY_DAYS) else {
            continue;
        };
        let mut tx = pool.begin().await?;
        let won = sqlx::query_scalar::<_, Uuid>(
            "INSERT INTO user_date_sent (date_id, year) VALUES ($1, $2) ON CONFLICT DO NOTHING RETURNING date_id")
            .bind(id).bind(crate::birthday::year_of(today, d)).fetch_optional(&mut *tx).await?;
        if won.is_some() {
            queued += sqlx::query(
                "INSERT INTO notifications (user_id, channel, kind, payload)
                 SELECT r.id, 'email', 'profile.date_soon', jsonb_build_object('date_id', $1::uuid, 'owner_id', $2::uuid)
                   FROM users r
                  WHERE r.id <> $2 AND r.deleted_at IS NULL AND r.blocked_at IS NULL
                    AND COALESCE(r.notification_prefs->>'inbox_social', 'true') <> 'false'
                    AND EXISTS (SELECT 1 FROM friendships f WHERE f.user_a = LEAST(r.id, $2) AND f.user_b = GREATEST(r.id, $2))
                    AND NOT EXISTS (SELECT 1 FROM user_blocks k WHERE (k.blocker_id = r.id AND k.blocked_id = $2) OR (k.blocker_id = $2 AND k.blocked_id = r.id))
                    AND ($3 <> 'close' OR EXISTS (SELECT 1 FROM close_friends c WHERE c.owner_id = $2 AND c.friend_id = r.id))")
                .bind(id).bind(owner).bind(&vis).execute(&mut *tx).await?.rows_affected();
        }
        tx.commit().await?;
    }
    Ok(queued)
}
