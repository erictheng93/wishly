//! 重要日子：CRUD、7 天前站內通知排程、寄前重驗。
mod common;
use chrono::NaiveDate;
use common::{befriend, call, ck, user};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

fn day(m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, m, d).unwrap()
}

async fn add(
    pool: &PgPool,
    owner: Uuid,
    md: &str,
    year: Option<i16>,
    rec: bool,
    vis: &str,
) -> Uuid {
    sqlx::query_scalar("INSERT INTO user_dates (user_id,title,month_day,year,recurring,visibility) VALUES ($1,'結婚紀念日',$2,$3,$4,$5::text::profile_vis) RETURNING id")
        .bind(owner).bind(md).bind(year).bind(rec).bind(vis).fetch_one(pool).await.unwrap()
}

async fn tick(pool: &PgPool, d: NaiveDate) -> u64 {
    wishly_api::user_dates::date_tick_at(pool, d).await.unwrap()
}

async fn got(pool: &PgPool) -> Vec<Uuid> {
    sqlx::query_scalar(
        "SELECT user_id FROM notifications WHERE kind='profile.date_soon' ORDER BY user_id",
    )
    .fetch_all(pool)
    .await
    .unwrap()
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn crud_limit_and_ownership(pool: PgPool) {
    let a = user(&pool, "A").await;
    let b = user(&pool, "B").await;
    let body = json!({"title":"紀念日","month_day":"05-20","year":null,"recurring":true,"visibility":"friends"});
    let (s, _, v) = call(
        &pool,
        "POST",
        "/me/dates",
        &ck(&a.token),
        Some(body.clone()),
    )
    .await;
    assert_eq!(s, 200);
    let id = v["id"].as_str().unwrap().to_string();
    for bad in [
        json!({"month_day":"02-30"}),
        json!({"title":""}),
        json!({"recurring":false}),
    ] {
        let mut x = body.clone();
        x.as_object_mut()
            .unwrap()
            .extend(bad.as_object().unwrap().clone());
        let (s, _, _) = call(&pool, "POST", "/me/dates", &ck(&a.token), Some(x)).await;
        assert_eq!(s, 422);
    }
    let (_, _, l) = call(&pool, "GET", "/me/dates", &ck(&a.token), None).await;
    assert_eq!(l["dates"][0]["id"], id);
    assert_eq!(l["dates"][0]["past"], false);
    // 非本人
    let (s, _, _) = call(
        &pool,
        "PATCH",
        &format!("/me/dates/{id}"),
        &ck(&b.token),
        Some(json!({"title":"x"})),
    )
    .await;
    assert_eq!(s, 404);
    let (s, _, _) = call(
        &pool,
        "DELETE",
        &format!("/me/dates/{id}"),
        &ck(&b.token),
        None,
    )
    .await;
    assert_eq!(s, 404);
    let (s, _, _) = call(
        &pool,
        "PATCH",
        &format!("/me/dates/{id}"),
        &ck(&a.token),
        Some(json!({"year":2020,"recurring":false})),
    )
    .await;
    assert_eq!(s, 200);
    let (_, _, l) = call(&pool, "GET", "/me/dates", &ck(&a.token), None).await;
    assert_eq!(l["dates"][0]["past"], true);
    // 上限 20
    for _ in 0..19 {
        add(&pool, a.id, "01-01", None, true, "friends").await;
    }
    let (s, _, v) = call(&pool, "POST", "/me/dates", &ck(&a.token), Some(body)).await;
    assert_eq!((s.as_u16(), v["code"].as_str()), (422, Some("LIMIT")));
    let (s, _, _) = call(
        &pool,
        "DELETE",
        &format!("/me/dates/{id}"),
        &ck(&a.token),
        None,
    )
    .await;
    assert_eq!(s, 200);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn notifies_friends_7_days_before(pool: PgPool) {
    let (o, f, _s) = (
        user(&pool, "O").await,
        user(&pool, "F").await,
        user(&pool, "S").await,
    );
    befriend(&pool, o.id, f.id).await;
    add(&pool, o.id, "05-20", None, true, "friends").await;
    assert_eq!(tick(&pool, day(5, 12)).await, 0); // 8 天前
    assert_eq!(tick(&pool, day(5, 13)).await, 1);
    assert_eq!(got(&pool).await, vec![f.id]);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn close_date_notifies_only_close(pool: PgPool) {
    let (o, f, c) = (
        user(&pool, "O").await,
        user(&pool, "F").await,
        user(&pool, "C").await,
    );
    befriend(&pool, o.id, f.id).await;
    befriend(&pool, o.id, c.id).await;
    sqlx::query("INSERT INTO close_friends (owner_id, friend_id) VALUES ($1,$2)")
        .bind(o.id)
        .bind(c.id)
        .execute(&pool)
        .await
        .unwrap();
    add(&pool, o.id, "05-20", None, true, "close").await;
    tick(&pool, day(5, 15)).await;
    assert_eq!(got(&pool).await, vec![c.id]);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn private_date_never_notifies(pool: PgPool) {
    let (o, f) = (user(&pool, "O").await, user(&pool, "F").await);
    befriend(&pool, o.id, f.id).await;
    add(&pool, o.id, "05-20", None, true, "private").await;
    assert_eq!(tick(&pool, day(5, 20)).await, 0);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn idempotent_same_year(pool: PgPool) {
    let (o, f) = (user(&pool, "O").await, user(&pool, "F").await);
    befriend(&pool, o.id, f.id).await;
    add(&pool, o.id, "05-20", None, true, "friends").await;
    assert_eq!(tick(&pool, day(5, 14)).await, 1);
    assert_eq!(tick(&pool, day(5, 14)).await, 0);
    assert_eq!(tick(&pool, day(5, 15)).await, 0);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn late_tick_still_sends_within_window(pool: PgPool) {
    let (o, f) = (user(&pool, "O").await, user(&pool, "F").await);
    befriend(&pool, o.id, f.id).await;
    add(&pool, o.id, "05-20", None, true, "friends").await;
    assert_eq!(tick(&pool, day(5, 17)).await, 1);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn one_off_past_not_sent(pool: PgPool) {
    let (o, f) = (user(&pool, "O").await, user(&pool, "F").await);
    befriend(&pool, o.id, f.id).await;
    add(&pool, o.id, "05-20", Some(2025), false, "friends").await; // 去年的一次性
    assert_eq!(tick(&pool, day(5, 15)).await, 0);
    add(&pool, o.id, "05-21", Some(2026), false, "friends").await;
    assert_eq!(tick(&pool, day(5, 15)).await, 1);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn edit_date_resets_sent(pool: PgPool) {
    let (o, f) = (user(&pool, "O").await, user(&pool, "F").await);
    befriend(&pool, o.id, f.id).await;
    let id = add(&pool, o.id, "05-20", None, true, "friends").await;
    assert_eq!(tick(&pool, day(5, 15)).await, 1);
    let (s, _, _) = call(
        &pool,
        "PATCH",
        &format!("/me/dates/{id}"),
        &ck(&o.token),
        Some(json!({"month_day":"05-21"})),
    )
    .await;
    assert_eq!(s, 200);
    let pending: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM notifications WHERE kind='profile.date_soon' AND status='pending'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(pending, 0);
    assert_eq!(tick(&pool, day(5, 15)).await, 1);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn blocked_recipient_skipped(pool: PgPool) {
    let (o, f, g) = (
        user(&pool, "O").await,
        user(&pool, "F").await,
        user(&pool, "G").await,
    );
    befriend(&pool, o.id, f.id).await;
    befriend(&pool, o.id, g.id).await;
    sqlx::query("INSERT INTO user_blocks (blocker_id, blocked_id) VALUES ($1,$2)")
        .bind(f.id)
        .bind(o.id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO user_blocks (blocker_id, blocked_id) VALUES ($1,$2)")
        .bind(o.id)
        .bind(g.id)
        .execute(&pool)
        .await
        .unwrap();
    add(&pool, o.id, "05-20", None, true, "friends").await;
    assert_eq!(tick(&pool, day(5, 15)).await, 0);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn inbox_social_off_skipped(pool: PgPool) {
    let (o, f) = (user(&pool, "O").await, user(&pool, "F").await);
    befriend(&pool, o.id, f.id).await;
    sqlx::query(
        "UPDATE users SET notification_prefs = '{\"inbox_social\":\"false\"}'::jsonb WHERE id=$1",
    )
    .bind(f.id)
    .execute(&pool)
    .await
    .unwrap();
    add(&pool, o.id, "05-20", None, true, "friends").await;
    assert_eq!(tick(&pool, day(5, 15)).await, 0);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn feb29_date_notifies_in_non_leap_year(pool: PgPool) {
    let (o, f) = (user(&pool, "O").await, user(&pool, "F").await);
    befriend(&pool, o.id, f.id).await;
    add(&pool, o.id, "02-29", None, true, "friends").await;
    assert_eq!(tick(&pool, day(2, 22)).await, 1);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn worker_cancels_if_made_private(pool: PgPool) {
    let (o, f) = (user(&pool, "O").await, user(&pool, "F").await);
    befriend(&pool, o.id, f.id).await;
    let id = add(&pool, o.id, "05-20", None, true, "friends").await;
    tick(&pool, day(5, 15)).await;
    sqlx::query("UPDATE user_dates SET visibility='private' WHERE id=$1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    wishly_api::notify::tick(&pool).await.unwrap();
    let st: String =
        sqlx::query_scalar("SELECT status::text FROM notifications WHERE kind='profile.date_soon'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(st, "cancelled");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn worker_writes_inbox_for_visible_date(pool: PgPool) {
    let (o, f) = (user(&pool, "O").await, user(&pool, "F").await);
    befriend(&pool, o.id, f.id).await;
    add(&pool, o.id, "05-20", None, true, "friends").await;
    tick(&pool, day(5, 15)).await;
    wishly_api::notify::tick(&pool).await.unwrap();
    let (t, l): (String, String) = sqlx::query_as(
        "SELECT inbox_title, inbox_link FROM notifications WHERE kind='profile.date_soon'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(t.contains("結婚紀念日") && l.starts_with("/u/"), "{t} {l}");
}
