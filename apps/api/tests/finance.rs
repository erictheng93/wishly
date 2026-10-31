//! 金流後台總覽（指標定義見 docs/superpowers/specs/2026-10-31-finance-dashboard-design.md）。
mod common;
use axum::http::StatusCode;
use common::{call, ck, grant, staff, topup_credit, user};
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

const TODAY0: &str =
    "((now() AT TIME ZONE 'Asia/Taipei')::date::timestamp AT TIME ZONE 'Asia/Taipei')";

async fn overview(pool: &PgPool, days: i32) -> Value {
    let s = staff(pool, "營運").await;
    let (st, _, b) = call(
        pool,
        "GET",
        &format!("/admin/finance/overview?days={days}"),
        &ck(&s.token),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{b}");
    b
}

/// 以 SQL 表達式設定該筆儲值的 paid_at。
async fn paid_at(pool: &PgPool, id: Uuid, expr: &str) {
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "UPDATE topups SET paid_at = {expr} WHERE id = $1"
    )))
    .bind(id)
    .execute(pool)
    .await
    .unwrap();
}

async fn set_status(pool: &PgPool, id: Uuid, status: &str) {
    sqlx::query("UPDATE topups SET status = $2::topup_status WHERE id = $1")
        .bind(id)
        .bind(status)
        .execute(pool)
        .await
        .unwrap();
}

async fn wallet_of(pool: &PgPool, u: Uuid) -> Uuid {
    sqlx::query_scalar("SELECT id FROM point_wallets WHERE user_id = $1")
        .bind(u)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn overview_requires_staff(pool: PgPool) {
    let u = user(&pool, "一般").await;
    let (st, ..) = call(&pool, "GET", "/admin/finance/overview", &ck(&u.token), None).await;
    assert_eq!(st, StatusCode::FORBIDDEN);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn overview_rejects_bad_days(pool: PgPool) {
    let s = staff(&pool, "營運").await;
    for d in [0, 2, 400] {
        let (st, ..) = call(
            &pool,
            "GET",
            &format!("/admin/finance/overview?days={d}"),
            &ck(&s.token),
            None,
        )
        .await;
        assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY, "days={d}");
    }
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn overview_empty_is_zero(pool: PgPool) {
    let s = staff(&pool, "營運").await;
    let (st, h, b) = call(
        &pool,
        "GET",
        "/admin/finance/overview?days=7",
        &ck(&s.token),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(h.get("cache-control").unwrap(), "private, no-store");
    assert_eq!(b["daily"].as_array().unwrap().len(), 7);
    assert!(b["daily"]
        .as_array()
        .unwrap()
        .iter()
        .all(|d| d["gross"] == 0));
    assert_eq!(b["kpis"]["cur"]["gross"], 0);
    assert_eq!(b["kpis"]["prev"]["payers"], 0);
    for k in ["disputed", "cash_refunds", "debts", "pending"] {
        assert_eq!(
            (
                b["todo"][k]["count"].as_i64(),
                b["todo"][k]["amount"].as_i64()
            ),
            (Some(0), Some(0)),
            "{k}"
        );
    }
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn gross_excludes_manual_grant(pool: PgPool) {
    let u = user(&pool, "甲").await;
    grant(&pool, u.id, 500).await;
    topup_credit(&pool, u.id, 100).await;
    let b = overview(&pool, 7).await;
    assert_eq!(b["kpis"]["cur"]["gross"], 100);
    assert_eq!(b["kpis"]["cur"]["payers"], 1);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn gross_keeps_charged_back_and_won(pool: PgPool) {
    let u = user(&pool, "甲").await;
    let a = topup_credit(&pool, u.id, 100).await;
    let c = topup_credit(&pool, u.id, 200).await;
    let w = topup_credit(&pool, u.id, 400).await;
    set_status(&pool, a, "charged_back").await;
    set_status(&pool, c, "disputed").await;
    set_status(&pool, w, "paid").await; // 勝訴後回到 paid
    let b = overview(&pool, 7).await;
    assert_eq!(b["kpis"]["cur"]["gross"], 700);
    assert_eq!(b["kpis"]["cur"]["payers"], 1);
    assert_eq!(b["todo"]["disputed"]["count"], 1);
    assert_eq!(b["todo"]["disputed"]["amount"], 200);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn first_time_vs_repeat(pool: PgPool) {
    let old = user(&pool, "舊客").await;
    let new = user(&pool, "新客").await;
    let o1 = topup_credit(&pool, old.id, 100).await;
    paid_at(&pool, o1, "now() - interval '100 days'").await;
    topup_credit(&pool, old.id, 100).await;
    topup_credit(&pool, new.id, 100).await;
    let b = overview(&pool, 7).await;
    assert_eq!(b["kpis"]["cur"]["payers"], 2);
    assert_eq!(b["kpis"]["cur"]["first_time"], 1);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn taipei_midnight_boundary(pool: PgPool) {
    let u = user(&pool, "甲").await;
    let before = topup_credit(&pool, u.id, 100).await;
    let after = topup_credit(&pool, u.id, 300).await;
    paid_at(&pool, before, &format!("{TODAY0} - interval '1 second'")).await;
    paid_at(&pool, after, &format!("{TODAY0} + interval '1 second'")).await;
    let b = overview(&pool, 7).await;
    let d = b["daily"].as_array().unwrap();
    assert_eq!(
        (d[5]["gross"].as_i64(), d[6]["gross"].as_i64()),
        (Some(100), Some(300))
    );
    // days=1 只含今日
    assert_eq!(overview(&pool, 1).await["kpis"]["cur"]["gross"], 300);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn prev_period_cuts_at_same_moment(pool: PgPool) {
    let u = user(&pool, "甲").await;
    let late = topup_credit(&pool, u.id, 100).await; // 昨天、但晚於「昨天此刻」
    let early = topup_credit(&pool, u.id, 50).await; // 昨天凌晨之後、早於此刻
    paid_at(&pool, late, &format!("{TODAY0} - interval '1 second'")).await;
    paid_at(
        &pool,
        early,
        &format!("{TODAY0} - interval '1 day' + interval '1 second'"),
    )
    .await;
    let b = overview(&pool, 1).await;
    assert_eq!(b["kpis"]["prev"]["gross"], 50);
    assert_eq!(b["kpis"]["cur"]["gross"], 0);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn pending_ignores_expired(pool: PgPool) {
    let u = user(&pool, "甲").await;
    grant(&pool, u.id, 1).await;
    let w = wallet_of(&pool, u.id).await;
    for (amt, exp) in [
        (100, "now() + interval '1 day'"),
        (900, "now() - interval '1 day'"),
    ] {
        let id = Uuid::new_v4();
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "INSERT INTO topups (id, wallet_id, merchant_trade_no, provider, method, amount, status, expires_at)
             VALUES ($1, $2, $3, 'ecpay', 'atm', $4, 'pending', {exp})"
        )))
        .bind(id).bind(w).bind(id.to_string()).bind(amt as i64)
        .execute(&pool).await.unwrap();
    }
    let b = overview(&pool, 7).await;
    assert_eq!(
        (
            b["todo"]["pending"]["count"].as_i64(),
            b["todo"]["pending"]["amount"].as_i64()
        ),
        (Some(1), Some(100))
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn todo_debts_nets_recovered(pool: PgPool) {
    let u = user(&pool, "甲").await;
    let t = topup_credit(&pool, u.id, 1000).await;
    let w = wallet_of(&pool, u.id).await;
    sqlx::query("INSERT INTO wallet_debts (wallet_id, topup_id, amount, recovered_points) VALUES ($1, $2, 1000, 400)")
        .bind(w).bind(t).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO cash_refund_requests (wallet_id, amount, status) VALUES ($1, 70, 'processing')")
        .bind(w).execute(&pool).await.unwrap();
    let b = overview(&pool, 7).await;
    assert_eq!(
        (
            b["todo"]["debts"]["count"].as_i64(),
            b["todo"]["debts"]["amount"].as_i64()
        ),
        (Some(1), Some(600))
    );
    assert_eq!(
        (
            b["todo"]["cash_refunds"]["count"].as_i64(),
            b["todo"]["cash_refunds"]["amount"].as_i64()
        ),
        (Some(1), Some(70))
    );
}
