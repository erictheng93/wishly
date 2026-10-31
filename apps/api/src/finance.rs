//! 金流後台（唯讀）：指標定義見 docs/superpowers/specs/2026-10-31-finance-dashboard-design.md「指標定義」。
//! 金額一律來自 `topups`（人工發放不算營收）；日界線 Asia/Taipei；統計不排除營運／demo／已刪除用戶。
use crate::{admin::Staff, error::AppError, AppState};
use axum::{
    extract::{Query, State},
    http::{header, StatusCode},
    response::Response,
    routing::get,
    Router,
};
use serde::Deserialize;
use serde_json::Value;

type R<T> = Result<T, AppError>;

pub fn routes() -> Router<AppState> {
    Router::new().route("/admin/finance/overview", get(overview))
}

#[derive(Deserialize)]
pub struct DaysQ {
    pub days: Option<i32>,
}

/// 只允許 1、7、30、90、365；預設 30。
pub fn parse_days(q: &DaysQ) -> R<i32> {
    let d = q.days.unwrap_or(30);
    if ![1, 7, 30, 90, 365].contains(&d) {
        return Err(AppError::invalid(
            "/days",
            "OUT_OF_RANGE",
            "days 須為 1、7、30、90 或 365",
        ));
    }
    Ok(d)
}

/// 共用區間（`$1` = days）：d0 為含今日的 N 天起點（台北日期）；前期為往前平移 N 天，
/// 結束於「現在減 N 天」，看今日時比的是昨天同一時刻以前。
pub const RANGE_CTE: &str = "b AS (SELECT (now() AT TIME ZONE 'Asia/Taipei')::date AS today),
         r AS (SELECT today, today - ($1 - 1) AS d0 FROM b),
         rg AS (SELECT today, d0,
                  d0::timestamp AT TIME ZONE 'Asia/Taipei' AS cur_start,
                  (d0::timestamp AT TIME ZONE 'Asia/Taipei') - make_interval(days => $1) AS prev_start,
                  now() - make_interval(days => $1) AS prev_end FROM r)";

pub fn json_resp(body: &Value) -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CACHE_CONTROL, "private, no-store")
        .header(header::CONTENT_TYPE, "application/json")
        .body(axum::body::Body::from(body.to_string()))
        .unwrap()
}

async fn overview(State(st): State<AppState>, _s: Staff, Query(q): Query<DaysQ>) -> R<Response> {
    let days = parse_days(&q)?;
    let sql = format!(
        "WITH {RANGE_CTE},
         -- 每個錢包有史以來第一筆付款時間（首次付費判斷）
         fp AS (SELECT wallet_id, min(paid_at) AS first_paid FROM topups WHERE paid_at IS NOT NULL GROUP BY wallet_id),
         cur AS (SELECT t.* FROM topups t, rg WHERE t.paid_at >= rg.cur_start),
         prev AS (SELECT t.* FROM topups t, rg WHERE t.paid_at >= rg.prev_start AND t.paid_at < rg.prev_end),
         daily AS (
           SELECT gs::date AS d, COALESCE(sum(c.amount), 0)::bigint AS gross
           FROM rg CROSS JOIN generate_series(rg.d0, rg.today, interval '1 day') gs
           LEFT JOIN cur c ON (c.paid_at AT TIME ZONE 'Asia/Taipei')::date = gs::date GROUP BY gs)
         SELECT jsonb_build_object(
           'range', (SELECT jsonb_build_object('from', d0, 'to', today, 'days', $1) FROM rg),
           'todo', jsonb_build_object(
             'disputed', (SELECT jsonb_build_object('count', count(*), 'amount', COALESCE(sum(amount), 0)::bigint) FROM topups WHERE status = 'disputed'),
             'cash_refunds', (SELECT jsonb_build_object('count', count(*), 'amount', COALESCE(sum(amount), 0)::bigint) FROM cash_refund_requests WHERE status IN ('requested', 'processing')),
             'debts', (SELECT jsonb_build_object('count', count(*), 'amount', COALESCE(sum(amount - recovered_points), 0)::bigint) FROM wallet_debts WHERE status = 'open'),
             'pending', (SELECT jsonb_build_object('count', count(*), 'amount', COALESCE(sum(amount), 0)::bigint) FROM topups WHERE status = 'pending' AND expires_at > now())),
           'kpis', jsonb_build_object(
             'cur', jsonb_build_object(
               'gross', (SELECT COALESCE(sum(amount), 0)::bigint FROM cur),
               'payers', (SELECT count(DISTINCT wallet_id) FROM cur),
               'first_time', (SELECT count(*) FROM fp, rg WHERE fp.first_paid >= rg.cur_start)),
             'prev', jsonb_build_object(
               'gross', (SELECT COALESCE(sum(amount), 0)::bigint FROM prev),
               'payers', (SELECT count(DISTINCT wallet_id) FROM prev),
               'first_time', (SELECT count(*) FROM fp, rg WHERE fp.first_paid >= rg.prev_start AND fp.first_paid < rg.prev_end))),
           'daily', (SELECT jsonb_agg(jsonb_build_object('date', d, 'gross', gross) ORDER BY d) FROM daily))"
    );
    let (v,): (Value,) = sqlx::query_as(sqlx::AssertSqlSafe(sql))
        .bind(days)
        .fetch_one(&st.pool)
        .await?;
    Ok(json_resp(&v))
}
