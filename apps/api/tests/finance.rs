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
    get(pool, "overview", days).await
}

async fn get(pool: &PgPool, ep: &str, days: i32) -> Value {
    let s = staff(pool, "營運").await;
    let (st, _, b) = call(
        pool,
        "GET",
        &format!("/admin/finance/{ep}?days={days}"),
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
    for d in ["0", "2", "400", "abc"] {
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

async fn email_of(pool: &PgPool, u: Uuid) -> String {
    sqlx::query_scalar("SELECT email FROM users WHERE id = $1")
        .bind(u)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn order(pool: &PgPool, u: Uuid, method: &str, status: &str, paid: bool) -> Uuid {
    grant(pool, u, 1).await;
    let w = wallet_of(pool, u).await;
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO topups (id, wallet_id, merchant_trade_no, provider, method, amount, status, expires_at, paid_at, provider_txn_id)
         VALUES ($1, $2, $3, 'ecpay', $4::topup_method, 100, $5::topup_status, now() + interval '1 day', CASE WHEN $6 THEN now() END, CASE WHEN $6 THEN $3 END)",
    )
    .bind(id).bind(w).bind(id.to_string()).bind(method).bind(status).bind(paid)
    .execute(pool).await.unwrap();
    id
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn payers_buckets_and_top_order(pool: PgPool) {
    // 25 人：金額 100 起每人 +100，另一人 20000
    for i in 1..=24i64 {
        let u = user(&pool, &format!("u{i}")).await;
        topup_credit(&pool, u.id, i * 100).await;
    }
    let big = user(&pool, "大戶").await;
    topup_credit(&pool, big.id, 20000).await;
    let b = get(&pool, "payers", 7).await;
    let keys: Vec<&str> = b["buckets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["key"].as_str().unwrap())
        .collect();
    assert_eq!(
        keys,
        ["lt500", "500_999", "1000_2999", "3000_9999", "gte10000"]
    );
    let cnt: Vec<i64> = b["buckets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["count"].as_i64().unwrap())
        .collect();
    assert_eq!(cnt, [4, 5, 15, 0, 1]); // 100-400 / 500-900 / 1000-2400 + 無 / 20000
    assert_eq!(b["payers"], 25);
    let top = b["top"].as_array().unwrap();
    assert_eq!(top.len(), 20);
    assert_eq!(top[0]["user"]["display_name"], "大戶");
    assert_eq!(top[0]["amount"], 20000);
    assert_eq!(top[1]["amount"], 2400);
    assert!(top[0]["user"]["email"].is_string());
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn payers_repeat_counts_history(pool: PgPool) {
    let a = user(&pool, "回購").await;
    let c = user(&pool, "單次").await;
    let old = topup_credit(&pool, a.id, 100).await;
    paid_at(&pool, old, "now() - interval '100 days'").await;
    topup_credit(&pool, a.id, 100).await;
    topup_credit(&pool, c.id, 100).await;
    let b = get(&pool, "payers", 7).await;
    assert_eq!(b["payers"], 2);
    assert_eq!(b["first_time"], 1);
    assert_eq!(b["repeat_rate"].as_f64(), Some(0.5));
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn payers_keeps_deleted_user(pool: PgPool) {
    let u = user(&pool, "甲").await;
    topup_credit(&pool, u.id, 300).await;
    sqlx::query("UPDATE users SET display_name = '已刪除的使用者', email = NULL, deleted_at = now(), anonymized_at = now() WHERE id = $1")
        .bind(u.id).execute(&pool).await.unwrap();
    let b = get(&pool, "payers", 7).await;
    assert_eq!(b["gross"], 300);
    assert_eq!(b["top"][0]["user"]["display_name"], "已刪除的使用者");
    assert!(b["top"][0]["user"]["email"].is_null());
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn payers_empty_rate_null(pool: PgPool) {
    let b = get(&pool, "payers", 7).await;
    assert!(b["repeat_rate"].is_null());
    assert_eq!(b["buckets"].as_array().unwrap().len(), 5);
    assert_eq!(b["top"].as_array().unwrap().len(), 0);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn channels_success_rate(pool: PgPool) {
    let u = user(&pool, "甲").await;
    order(&pool, u.id, "card", "paid", true).await;
    order(&pool, u.id, "card", "failed", false).await;
    order(&pool, u.id, "card", "expired", false).await;
    order(&pool, u.id, "card", "pending", false).await;
    let b = get(&pool, "channels", 7).await;
    let card = &b["channels"][0];
    assert_eq!(card["method"], "card");
    assert_eq!(card["gross"], 100);
    assert_eq!(card["count"], 1);
    assert_eq!(card["avg"].as_f64(), Some(100.0));
    assert_eq!(card["payers"], 1);
    let r = card["success_rate"].as_f64().unwrap();
    assert!((r - 1.0 / 3.0).abs() < 1e-6, "{r}");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn channels_hide_unused_wallet_methods(pool: PgPool) {
    let b = get(&pool, "channels", 7).await;
    let m: Vec<&str> = b["channels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["method"].as_str().unwrap())
        .collect();
    for k in ["card", "cvs_code", "cvs_barcode", "atm"] {
        assert!(m.contains(&k), "{k}");
    }
    assert!(!m.contains(&"line_pay") && !m.contains(&"apple_pay"));
    let u = user(&pool, "甲").await;
    order(&pool, u.id, "line_pay", "paid", true).await;
    let b = get(&pool, "channels", 7).await;
    assert_eq!(b["channels"][0]["method"], "line_pay"); // gross 降冪
    assert_eq!(b["channels"].as_array().unwrap().len(), 5);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn channels_empty_rates_null(pool: PgPool) {
    let b = get(&pool, "channels", 7).await;
    for c in b["channels"].as_array().unwrap() {
        assert!(c["avg"].is_null() && c["success_rate"].is_null(), "{c}");
        assert_eq!(c["gross"], 0);
    }
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn risk_liability_splits(pool: PgPool) {
    let u = user(&pool, "甲").await;
    topup_credit(&pool, u.id, 100).await;
    grant(&pool, u.id, 50).await;
    // 認捐 30（扣自 adjustment 批次，已不在餘額內）
    let owner = user(&pool, "主").await;
    let wl = common::wishlist(&pool, owner.id, Default::default()).await;
    let it = common::crowdfund_item(&pool, wl.id, 1000).await;
    let app = common::test_app(&pool);
    let (st, ..) = common::pledge(&app, it, &u.token, 30).await;
    assert_eq!(st, StatusCode::CREATED);
    let b = get(&pool, "risk", 7).await;
    let l = &b["liability"];
    assert_eq!(l["refundable"], 100);
    assert_eq!(l["non_refundable"], 20);
    assert_eq!(l["unused"], 120);
    assert_eq!(l["pledged"], 30);
    assert_eq!(l["total"], 150);
    let sum: i64 = sqlx::query_scalar("SELECT sum(balance)::bigint FROM point_wallets")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(l["unused"], sum);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn risk_no_email_leak(pool: PgPool) {
    let u = user(&pool, "甲").await;
    let t = topup_credit(&pool, u.id, 100).await;
    set_status(&pool, t, "disputed").await;
    let email = email_of(&pool, u.id).await;
    for ep in ["risk", "channels", "overview"] {
        let b = get(&pool, ep, 7).await;
        assert!(!b.to_string().contains(&email), "{ep}");
    }
    let b = get(&pool, "risk", 7).await;
    assert_eq!(b["disputed"][0]["display_name"], "甲");
    assert_eq!(b["disputed"][0]["amount"], 100);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn risk_refund_and_chargeback_windows(pool: PgPool) {
    let u = user(&pool, "甲").await;
    let w = {
        grant(&pool, u.id, 1).await;
        wallet_of(&pool, u.id).await
    };
    for (amt, when) in [(70, "now()"), (900, "now() - interval '60 days'")] {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "INSERT INTO cash_refund_requests (wallet_id, amount, status, completed_at) VALUES ($1, {amt}, 'completed', {when})"
        ))).bind(w).execute(&pool).await.unwrap();
    }
    let cb_in = topup_credit(&pool, u.id, 200).await;
    let cb_out = topup_credit(&pool, u.id, 800).await;
    set_status(&pool, cb_in, "charged_back").await;
    set_status(&pool, cb_out, "charged_back").await;
    sqlx::query("UPDATE topups SET updated_at = now() - interval '60 days' WHERE id = $1")
        .bind(cb_out)
        .execute(&pool)
        .await
        .unwrap();
    let b = get(&pool, "risk", 7).await;
    assert_eq!(
        (
            b["refunds"]["count"].as_i64(),
            b["refunds"]["amount"].as_i64()
        ),
        (Some(1), Some(70))
    );
    assert_eq!(
        (
            b["chargebacks"]["count"].as_i64(),
            b["chargebacks"]["amount"].as_i64()
        ),
        (Some(1), Some(200))
    );
    assert_eq!(b["loss_rate"].as_f64(), Some(0.27)); // (70+200)/1000
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn risk_loss_rate_null_when_no_gross(pool: PgPool) {
    let b = get(&pool, "risk", 7).await;
    assert!(b["loss_rate"].is_null());
    assert_eq!(b["liability"]["total"], 0);
    assert_eq!(b["frozen_wallets"], 0);
    assert_eq!(b["invoices_pending"]["count"], 0);
    assert_eq!(b["disputed"].as_array().unwrap().len(), 0);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn risk_frozen_and_invoices(pool: PgPool) {
    let u = user(&pool, "甲").await;
    let paid = topup_credit(&pool, u.id, 100).await;
    let unpaid = order(&pool, u.id, "atm", "pending", false).await;
    for (id, amt) in [(paid, 100), (unpaid, 100)] {
        sqlx::query(
            "INSERT INTO invoices (source_type, source_id, amount) VALUES ('topup', $1, $2)",
        )
        .bind(id)
        .bind(amt as i64)
        .execute(&pool)
        .await
        .unwrap();
    }
    sqlx::query("UPDATE point_wallets SET status = 'frozen'")
        .execute(&pool)
        .await
        .unwrap();
    let b = get(&pool, "risk", 7).await;
    assert_eq!(b["frozen_wallets"], 1);
    assert_eq!(
        (
            b["invoices_pending"]["count"].as_i64(),
            b["invoices_pending"]["amount"].as_i64()
        ),
        (Some(1), Some(100))
    );
}
