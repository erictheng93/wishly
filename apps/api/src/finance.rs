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
    Router::new()
        .route("/admin/finance/overview", get(overview))
        .route("/admin/finance/payers", get(payers))
        .route("/admin/finance/channels", get(channels))
        .route("/admin/finance/risk", get(risk))
}

#[derive(Deserialize)]
pub struct DaysQ {
    // 用字串接，非數字也走 422 OUT_OF_RANGE 而非 Query 的 400
    pub days: Option<String>,
}

/// 只允許 1、7、30、90、365；預設 30。
pub fn parse_days(q: &DaysQ) -> R<i32> {
    let d = match q.days.as_deref() {
        None => 30,
        Some(v) => v.parse::<i32>().unwrap_or(-1),
    };
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

/// 付費用戶：以錢包為單位，金額與筆數只算區間內。
async fn payers(State(st): State<AppState>, _s: Staff, Query(q): Query<DaysQ>) -> R<Response> {
    let days = parse_days(&q)?;
    let sql = format!(
        "WITH {RANGE_CTE},
         cur AS (SELECT t.* FROM topups t, rg WHERE t.paid_at >= rg.cur_start),
         pw AS (SELECT wallet_id, sum(amount)::bigint AS amount, count(*) AS cnt,
                       min(paid_at) AS first_paid_at, max(paid_at) AS last_paid_at
                FROM cur GROUP BY wallet_id),
         hist AS (SELECT wallet_id, count(*) AS n, min(paid_at) AS first_paid
                  FROM topups WHERE paid_at IS NOT NULL GROUP BY wallet_id),
         bk(ord, key) AS (VALUES (1, 'lt500'), (2, '500_999'), (3, '1000_2999'), (4, '3000_9999'), (5, 'gte10000')),
         pb AS (SELECT CASE WHEN amount < 500 THEN 1 WHEN amount < 1000 THEN 2 WHEN amount < 3000 THEN 3
                            WHEN amount < 10000 THEN 4 ELSE 5 END AS ord, amount FROM pw)
         SELECT jsonb_build_object(
           'range', (SELECT jsonb_build_object('from', d0, 'to', today, 'days', $1) FROM rg),
           'gross', (SELECT COALESCE(sum(amount), 0)::bigint FROM pw),
           'payers', (SELECT count(*) FROM pw),
           'first_time', (SELECT count(*) FROM hist, rg WHERE hist.first_paid >= rg.cur_start),
           'repeat_rate', (SELECT CASE WHEN count(*) = 0 THEN NULL
                                  ELSE (count(*) FILTER (WHERE h.n >= 2))::float8 / count(*) END
                           FROM pw JOIN hist h USING (wallet_id)),
           'buckets', (SELECT jsonb_agg(jsonb_build_object('key', bk.key,
                         'count', (SELECT count(*) FROM pb WHERE pb.ord = bk.ord),
                         'amount', (SELECT COALESCE(sum(amount), 0)::bigint FROM pb WHERE pb.ord = bk.ord))
                       ORDER BY bk.ord) FROM bk),
           'top', COALESCE((SELECT jsonb_agg(jsonb_build_object(
                         'user', jsonb_build_object('id', u.id, 'display_name', u.display_name, 'email', u.email),
                         'amount', x.amount, 'count', x.cnt,
                         'first_paid_at', x.first_paid_at, 'last_paid_at', x.last_paid_at)
                       ORDER BY x.amount DESC, x.wallet_id)
                FROM (SELECT * FROM pw ORDER BY amount DESC, wallet_id LIMIT 20) x
                JOIN point_wallets w ON w.id = x.wallet_id JOIN users u ON u.id = w.user_id), '[]'::jsonb))"
    );
    let (v,): (Value,) = sqlx::query_as(sqlx::AssertSqlSafe(sql))
        .bind(days)
        .fetch_one(&st.pool)
        .await?;
    Ok(json_resp(&v))
}

/// 渠道比較：固定四種渠道必出現，line_pay / apple_pay 有資料才出現。
async fn channels(State(st): State<AppState>, _s: Staff, Query(q): Query<DaysQ>) -> R<Response> {
    let days = parse_days(&q)?;
    let sql = format!(
        "WITH {RANGE_CTE},
         paid AS (SELECT t.method::text AS m, sum(amount)::bigint AS gross, count(*) AS cnt,
                         count(DISTINCT wallet_id) AS payers
                  FROM topups t, rg WHERE t.paid_at >= rg.cur_start GROUP BY 1),
         ord AS (SELECT t.method::text AS m, count(*) FILTER (WHERE t.paid_at IS NOT NULL) AS ok, count(*) AS done
                 FROM topups t, rg WHERE t.created_at >= rg.cur_start AND t.status <> 'pending' GROUP BY 1),
         ms AS (SELECT m FROM (VALUES ('card'), ('cvs_code'), ('cvs_barcode'), ('atm')) v(m)
                UNION SELECT m FROM paid UNION SELECT m FROM ord),
         ch AS (SELECT ms.m, COALESCE(p.gross, 0) AS gross, COALESCE(p.cnt, 0) AS cnt, COALESCE(p.payers, 0) AS payers,
                       o.ok, o.done FROM ms LEFT JOIN paid p USING (m) LEFT JOIN ord o USING (m))
         SELECT jsonb_build_object(
           'range', (SELECT jsonb_build_object('from', d0, 'to', today, 'days', $1) FROM rg),
           'channels', (SELECT jsonb_agg(jsonb_build_object('method', m, 'gross', gross, 'count', cnt,
                          'avg', CASE WHEN cnt = 0 THEN NULL ELSE round(gross::numeric / cnt, 2) END,
                          'success_rate', CASE WHEN COALESCE(done, 0) = 0 THEN NULL ELSE ok::float8 / done END,
                          'payers', payers) ORDER BY gross DESC, m) FROM ch))"
    );
    let (v,): (Value,) = sqlx::query_as(sqlx::AssertSqlSafe(sql))
        .bind(days)
        .fetch_one(&st.pool)
        .await?;
    Ok(json_resp(&v))
}

/// 風險與負債：負債 = 未使用點數（餘額，依批次拆分）+ 認捐中（已自餘額扣除，不重複）。
async fn risk(State(st): State<AppState>, _s: Staff, Query(q): Query<DaysQ>) -> R<Response> {
    let days = parse_days(&q)?;
    let sql = format!(
        "WITH {RANGE_CTE},
         gross AS (SELECT COALESCE(sum(t.amount), 0)::bigint AS g FROM topups t, rg WHERE t.paid_at >= rg.cur_start),
         rf AS (SELECT count(*) AS c, COALESCE(sum(c.amount), 0)::bigint AS a
                FROM cash_refund_requests c, rg WHERE c.status = 'completed' AND c.completed_at >= rg.cur_start),
         cb AS (SELECT count(*) AS c, COALESCE(sum(t.amount), 0)::bigint AS a
                FROM topups t, rg WHERE t.status = 'charged_back' AND t.updated_at >= rg.cur_start),
         lot AS (SELECT COALESCE(sum(remaining_points) FILTER (WHERE cash_refundable), 0)::bigint AS r,
                        COALESCE(sum(remaining_points) FILTER (WHERE NOT cash_refundable), 0)::bigint AS n FROM point_lots),
         pl AS (SELECT COALESCE(sum(points - refunded_points), 0)::bigint AS p FROM contributions WHERE status = 'pledged')
         SELECT jsonb_build_object(
           'range', (SELECT jsonb_build_object('from', d0, 'to', today, 'days', $1) FROM rg),
           'liability', (SELECT jsonb_build_object('total', lot.r + lot.n + pl.p, 'unused', lot.r + lot.n,
                           'refundable', lot.r, 'non_refundable', lot.n, 'pledged', pl.p) FROM lot, pl),
           'refunds', (SELECT jsonb_build_object('count', c, 'amount', a) FROM rf),
           'chargebacks', (SELECT jsonb_build_object('count', c, 'amount', a) FROM cb),
           'loss_rate', (SELECT CASE WHEN gross.g = 0 THEN NULL ELSE (rf.a + cb.a)::float8 / gross.g END FROM gross, rf, cb),
           'disputed', COALESCE((SELECT jsonb_agg(jsonb_build_object('id', d.id, 'amount', d.amount, 'method', d.method,
                           'paid_at', d.paid_at, 'display_name', d.display_name) ORDER BY d.paid_at DESC, d.id)
                FROM (SELECT t.id, t.amount, t.method, t.paid_at, u.display_name FROM topups t
                      JOIN point_wallets w ON w.id = t.wallet_id JOIN users u ON u.id = w.user_id
                      WHERE t.status = 'disputed' ORDER BY t.paid_at DESC, t.id LIMIT 50) d), '[]'::jsonb),
           'cash_refunds_open', (SELECT jsonb_build_object('count', count(*), 'amount', COALESCE(sum(amount), 0)::bigint)
                FROM cash_refund_requests WHERE status IN ('requested', 'processing')),
           'debts_open', (SELECT jsonb_build_object('count', count(*), 'amount', COALESCE(sum(amount - recovered_points), 0)::bigint)
                FROM wallet_debts WHERE status = 'open'),
           'frozen_wallets', (SELECT count(*) FROM point_wallets WHERE status = 'frozen'),
           'invoices_pending', (SELECT jsonb_build_object('count', count(*), 'amount', COALESCE(sum(i.amount), 0)::bigint)
                FROM invoices i JOIN topups t ON t.id = i.source_id
                WHERE i.source_type = 'topup' AND i.status = 'pending' AND t.paid_at IS NOT NULL))"
    );
    let (v,): (Value,) = sqlx::query_as(sqlx::AssertSqlSafe(sql))
        .bind(days)
        .fetch_one(&st.pool)
        .await?;
    Ok(json_resp(&v))
}
