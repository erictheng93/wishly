//! user_activity_days：CurrentUser 寫入點、保存期限、刪帳、/admin/analytics 的活躍與留存。
mod common;
use common::*;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

const TODAY: &str = "(now() AT TIME ZONE 'Asia/Taipei')::date";

async fn days_of(pool: &PgPool, uid: Uuid) -> Vec<String> {
    sqlx::query_scalar("SELECT day::text FROM user_activity_days WHERE user_id = $1 ORDER BY day")
        .bind(uid)
        .fetch_all(pool)
        .await
        .unwrap()
}

/// 台北今天 + k 天（SQL 計算，不用 Rust 本地時間）
async fn day(pool: &PgPool, k: i32) -> String {
    sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT ({TODAY} + $1)::text")))
        .bind(k)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn seed(pool: &PgPool, uid: Uuid, k: i32) {
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "INSERT INTO user_activity_days (user_id, day) VALUES ($1, {TODAY} + $2)"
    )))
    .bind(uid)
    .bind(k)
    .execute(pool)
    .await
    .unwrap();
}

async fn last_seen_age_secs(pool: &PgPool, token: &str) -> f64 {
    sqlx::query_scalar("SELECT extract(epoch FROM now() - last_seen_at)::float8 FROM sessions WHERE token_hash = $1")
        .bind(wishly_api::session::hash_token(token))
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn first_request_writes_one_row_for_taipei_today(pool: PgPool) {
    let u = user(&pool, "A").await;
    assert!(days_of(&pool, u.id).await.is_empty());
    let (st, _, _) = call(&pool, "GET", "/me", &ck(&u.token), None).await;
    assert_eq!(st, 200);
    assert_eq!(days_of(&pool, u.id).await, vec![day(&pool, 0).await]);
    for _ in 0..3 {
        call(&pool, "GET", "/me", &ck(&u.token), None).await;
    }
    assert_eq!(days_of(&pool, u.id).await.len(), 1);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn fresh_session_but_missing_today_row_reinserts(pool: PgPool) {
    let u = user(&pool, "A").await;
    call(&pool, "GET", "/me", &ck(&u.token), None).await;
    sqlx::query("DELETE FROM user_activity_days WHERE user_id = $1")
        .bind(u.id)
        .execute(&pool)
        .await
        .unwrap();
    // last_seen_at 剛更新（不到 5 分鐘），仍須因 NOT EXISTS 補寫
    let (st, _, _) = call(&pool, "GET", "/me", &ck(&u.token), None).await;
    assert_eq!(st, 200);
    assert_eq!(
        days_of(&pool, u.id).await,
        vec![day(&pool, 0).await],
        "NOT EXISTS clause missing: today's row not re-inserted"
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn stale_session_with_existing_row_still_advances_last_seen(pool: PgPool) {
    let u = user(&pool, "A").await;
    seed(&pool, u.id, 0).await;
    sqlx::query(
        "UPDATE sessions SET last_seen_at = now() - interval '10 minutes' WHERE user_id = $1",
    )
    .bind(u.id)
    .execute(&pool)
    .await
    .unwrap();
    let (st, _, _) = call(&pool, "GET", "/me", &ck(&u.token), None).await;
    assert_eq!(st, 200);
    let age = last_seen_age_secs(&pool, &u.token).await;
    assert!(
        age < 60.0,
        "ON CONFLICT DO NOTHING missing: last_seen_at not advanced (age {age}s)"
    );
    assert_eq!(days_of(&pool, u.id).await.len(), 1);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn blocked_or_revoked_session_writes_nothing(pool: PgPool) {
    let b = user(&pool, "B").await;
    sqlx::query("UPDATE users SET blocked_at = now() WHERE id = $1")
        .bind(b.id)
        .execute(&pool)
        .await
        .unwrap();
    let (st, _, _) = call(&pool, "GET", "/me", &ck(&b.token), None).await;
    assert_eq!(st, 403);
    assert!(days_of(&pool, b.id).await.is_empty());

    let r = user(&pool, "R").await;
    sqlx::query("UPDATE sessions SET revoked_at = now() WHERE user_id = $1")
        .bind(r.id)
        .execute(&pool)
        .await
        .unwrap();
    let (st, _, _) = call(&pool, "GET", "/me", &ck(&r.token), None).await;
    assert_eq!(st, 401);
    assert!(days_of(&pool, r.id).await.is_empty());
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn cleanup_keeps_400_days_and_drops_older(pool: PgPool) {
    let u = user(&pool, "A").await;
    for k in [-401, -400, -399, 0] {
        seed(&pool, u.id, k).await;
    }
    wishly_api::ratelimit::cleanup(&pool).await.unwrap();
    // 條件為 day < today-400：第 400 天前保留，第 401 天前刪除
    assert_eq!(
        days_of(&pool, u.id).await,
        vec![
            day(&pool, -400).await,
            day(&pool, -399).await,
            day(&pool, 0).await
        ]
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn account_deletion_removes_activity_rows(pool: PgPool) {
    let u = user(&pool, "A").await;
    let other = user(&pool, "O").await;
    call(&pool, "GET", "/me", &ck(&u.token), None).await;
    seed(&pool, u.id, -5).await;
    seed(&pool, other.id, -5).await;
    assert_eq!(days_of(&pool, u.id).await.len(), 2);
    let (st, _, b) = call(
        &pool,
        "DELETE",
        "/me",
        &ck(&u.token),
        Some(json!({"confirm": "DELETE"})),
    )
    .await;
    assert_eq!(st, 200, "{b}");
    assert!(days_of(&pool, u.id).await.is_empty());
    assert_eq!(days_of(&pool, other.id).await.len(), 1);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn analytics_active_windows_and_daily(pool: PgPool) {
    let s = staff(&pool, "營運").await;
    let u: Vec<Uuid> = {
        let mut v = vec![];
        for i in 0..6 {
            v.push(user(&pool, &format!("u{i}")).await.id);
        }
        v
    };
    for (i, k) in [
        (0, 0),
        (1, -3),
        (2, -10),
        (3, 0),
        (3, -3),
        (3, -10),
        (4, -6),
        (5, -7),
    ] {
        seed(&pool, u[i], k).await;
    }
    seed(&pool, s.id, -1).await; // staff 不計
    let (st, _, b) = call(&pool, "GET", "/admin/analytics", &ck(&s.token), None).await;
    assert_eq!(st, 200, "{b}");
    // d1=今天；d7=今天起 7 個台北日（-6..0）；d30 含 -10
    assert_eq!(b["active"]["d1"], 2);
    assert_eq!(b["active"]["d7"], 4);
    assert_eq!(b["active"]["d30"], 6);
    let daily = b["daily"].as_array().unwrap();
    let mut expect = std::collections::HashMap::new();
    for (k, n) in [(0, 2), (-3, 2), (-6, 1), (-7, 1), (-10, 2)] {
        expect.insert(day(&pool, k).await, n);
    }
    for row in daily {
        let d = row["date"].as_str().unwrap();
        assert_eq!(
            row["active_users"],
            *expect.get(d).unwrap_or(&0),
            "date {d}"
        );
    }
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn analytics_retention_cohorts(pool: PgPool) {
    let s = staff(&pool, "營運").await;
    // (註冊距今天數, 活躍相對註冊日的天數)
    let specs: [(i32, &[i32]); 7] = [
        (-20, &[1, 7]),  // A：d1、d7 皆留存；d30 視窗未結束
        (-20, &[2, 14]), // B：活躍都在視窗外
        (-1, &[2]),      // C：d1 視窗在今天，尚未結束，不入 cohort
        (-2, &[1]),      // D：僅 d1 cohort 且留存
        (-70, &[30]),    // E：d30 留存
        (-70, &[60]),    // F：d30 視窗外
        (-100, &[1]),    // G：註冊在區間（90 天）之前
    ];
    for (reg, acts) in specs {
        let u = user(&pool, "x").await;
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "UPDATE users SET created_at = (({TODAY} + $2)::timestamp + interval '12 hours') AT TIME ZONE 'Asia/Taipei' WHERE id = $1"
        )))
        .bind(u.id)
        .bind(reg)
        .execute(&pool)
        .await
        .unwrap();
        for a in acts.iter().filter(|a| reg + **a <= 0) {
            seed(&pool, u.id, reg + a).await;
        }
    }
    // staff 同條件也不入 cohort
    let st_u = staff(&pool, "S2").await;
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "UPDATE users SET created_at = (({TODAY} - 70)::timestamp + interval '12 hours') AT TIME ZONE 'Asia/Taipei' WHERE id = $1"
    )))
    .bind(st_u.id)
    .execute(&pool)
    .await
    .unwrap();
    seed(&pool, st_u.id, -69).await;

    let (st, _, b) = call(
        &pool,
        "GET",
        "/admin/analytics?days=90",
        &ck(&s.token),
        None,
    )
    .await;
    assert_eq!(st, 200, "{b}");
    assert_eq!(b["retention"]["d1"], json!({"cohort": 5, "retained": 2}));
    assert_eq!(b["retention"]["d7"], json!({"cohort": 4, "retained": 1}));
    assert_eq!(b["retention"]["d30"], json!({"cohort": 2, "retained": 1}));
}
