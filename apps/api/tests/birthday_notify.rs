//! 生日清單通知（D65）：API 規則、排程 birthday_tick_at、寄前重查。
mod common;
use chrono::NaiveDate;
use common::{befriend, call, ck, user, wishlist, ListOpts, TestUser};
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

fn day(m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, m, d).unwrap()
}

async fn set_birthday(pool: &PgPool, u: Uuid, b: &str, vis: &str) {
    sqlx::query(
        "UPDATE users SET birthday=$2, birthday_visibility=$3::text::share_level WHERE id=$1",
    )
    .bind(u)
    .bind(b)
    .bind(vis)
    .execute(pool)
    .await
    .unwrap();
}

async fn enable(pool: &PgPool, wid: Uuid) {
    sqlx::query("UPDATE wishlists SET birthday_notify=true WHERE id=$1")
        .bind(wid)
        .execute(pool)
        .await
        .unwrap();
}

async fn queued(pool: &PgPool, kind: &str) -> Vec<Uuid> {
    sqlx::query_scalar("SELECT user_id FROM notifications WHERE kind=$1 ORDER BY user_id")
        .bind(kind)
        .fetch_all(pool)
        .await
        .unwrap()
}

async fn patch(pool: &PgPool, t: &TestUser, id: Uuid, b: Value) -> (u16, Value) {
    let (s, _, v) = call(
        pool,
        "PATCH",
        &format!("/wishlists/{id}"),
        &ck(&t.token),
        Some(b),
    )
    .await;
    (s.as_u16(), v)
}

fn code(v: &Value) -> (&str, &str) {
    (
        v["errors"][0]["pointer"].as_str().unwrap(),
        v["errors"][0]["code"].as_str().unwrap(),
    )
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn api_rules(pool: PgPool) {
    let o = user(&pool, "壽星").await;
    let a = wishlist(&pool, o.id, ListOpts::default()).await;
    let b = wishlist(&pool, o.id, ListOpts::default()).await;
    // 缺生日
    let (s, v) = patch(&pool, &o, a.id, json!({"birthday_notify": true})).await;
    assert_eq!(
        (s, code(&v)),
        (422, ("/birthday_notify", "BIRTHDAY_REQUIRED"))
    );
    // 生日 private
    set_birthday(&pool, o.id, "05-01", "private").await;
    let (s, v) = patch(&pool, &o, a.id, json!({"birthday_notify": true})).await;
    assert_eq!((s, code(&v).1), (422, "BIRTHDAY_PRIVATE"));
    set_birthday(&pool, o.id, "05-01", "friends").await;
    // 清單 private
    let (s, v) = patch(
        &pool,
        &o,
        a.id,
        json!({"birthday_notify": true, "visibility": "private"}),
    )
    .await;
    assert_eq!((s, code(&v).1), (422, "LIST_PRIVATE"));
    // 成功；第二份開啟時第一份自動關
    let (s, v) = patch(&pool, &o, a.id, json!({"birthday_notify": true})).await;
    assert_eq!((s, v["birthday_notify"].clone()), (200, json!(true)));
    let (s, _) = patch(&pool, &o, b.id, json!({"birthday_notify": true})).await;
    assert_eq!(s, 200);
    let on: Vec<Uuid> =
        sqlx::query_scalar("SELECT id FROM wishlists WHERE owner_id=$1 AND birthday_notify")
            .bind(o.id)
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(on, vec![b.id]);
    // 改 private 自動關
    let (s, v) = patch(&pool, &o, b.id, json!({"visibility": "private"})).await;
    assert_eq!((s, v["birthday_notify"].clone()), (200, json!(false)));
    // 建立時直接開
    let (s, _, v) = call(
        &pool,
        "POST",
        "/wishlists",
        &ck(&o.token),
        Some(json!({"type":"registry","title":"生日","birthday_notify":true})),
    )
    .await;
    assert_eq!(
        (s.as_u16(), v["birthday_notify"].clone()),
        (201, json!(true))
    );
    // 生日改 private：清單全關、pending 朋友通知取消
    let f = user(&pool, "友").await;
    sqlx::query("INSERT INTO notifications(user_id,channel,kind,payload) VALUES($1,'email','birthday.friend',jsonb_build_object('owner_id',$2::uuid))").bind(f.id).bind(o.id).execute(&pool).await.unwrap();
    let (s, _, _) = call(
        &pool,
        "PATCH",
        "/me",
        &ck(&o.token),
        Some(json!({"birthday_visibility": "private"})),
    )
    .await;
    assert_eq!(s.as_u16(), 200);
    let n: i64 =
        sqlx::query_scalar("SELECT count(*) FROM wishlists WHERE owner_id=$1 AND birthday_notify")
            .bind(o.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    let st: String =
        sqlx::query_scalar("SELECT status::text FROM notifications WHERE kind='birthday.friend'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!((n, st.as_str()), (0, "cancelled"));
    // 偏好
    let (s, _, _) = call(
        &pool,
        "PATCH",
        "/me",
        &ck(&o.token),
        Some(json!({"notification_prefs": {"email_social": false}})),
    )
    .await;
    assert_eq!(s.as_u16(), 200);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn tick_recipients_and_idempotency(pool: PgPool) {
    let o = user(&pool, "壽星").await;
    set_birthday(&pool, o.id, "10-05", "friends").await;
    let w = wishlist(
        &pool,
        o.id,
        ListOpts {
            visibility: Some("selected"),
            ..Default::default()
        },
    )
    .await;
    enable(&pool, w.id).await;
    let (f_in, f_out, f_off, stranger) = (
        user(&pool, "a").await,
        user(&pool, "b").await,
        user(&pool, "c").await,
        user(&pool, "d").await,
    );
    for f in [&f_in, &f_out, &f_off] {
        befriend(&pool, o.id, f.id).await;
    }
    for f in [&f_in, &f_off, &stranger] {
        sqlx::query("INSERT INTO wishlist_allowed_users(wishlist_id,user_id) VALUES($1,$2)")
            .bind(w.id)
            .bind(f.id)
            .execute(&pool)
            .await
            .unwrap();
    }
    sqlx::query("UPDATE users SET notification_prefs='{\"email_social\": false, \"inbox_social\": false}' WHERE id=$1")
        .bind(f_off.id)
        .execute(&pool)
        .await
        .unwrap();

    // 8 天前：不寄；7 天前：只寄 selected 且是好友、沒關偏好者
    assert_eq!(
        wishly_api::birthday::birthday_tick_at(&pool, day(9, 27))
            .await
            .unwrap(),
        0
    );
    wishly_api::birthday::birthday_tick_at(&pool, day(9, 28))
        .await
        .unwrap();
    assert_eq!(queued(&pool, "birthday.friend").await, vec![f_in.id]);
    // 同年再跑（含補跑日）不重排
    wishly_api::birthday::birthday_tick_at(&pool, day(9, 28))
        .await
        .unwrap();
    wishly_api::birthday::birthday_tick_at(&pool, day(10, 3))
        .await
        .unwrap();
    assert_eq!(
        queued(&pool, "birthday.friend").await.len(),
        1,
        "birthday friend notification queued twice"
    );

    // public 清單寄全部好友
    sqlx::query("UPDATE wishlists SET visibility='public' WHERE id=$1")
        .bind(w.id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE users SET birthday_notified_year=NULL WHERE id=$1")
        .bind(o.id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM notifications")
        .execute(&pool)
        .await
        .unwrap();
    wishly_api::birthday::birthday_tick_at(&pool, day(10, 1))
        .await
        .unwrap();
    let mut want = vec![f_in.id, f_out.id];
    want.sort();
    assert_eq!(queued(&pool, "birthday.friend").await, want);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn tick_cross_year(pool: PgPool) {
    let o = user(&pool, "壽星").await;
    let f = user(&pool, "友").await;
    befriend(&pool, o.id, f.id).await;
    set_birthday(&pool, o.id, "01-02", "public").await;
    let w = wishlist(&pool, o.id, ListOpts::default()).await;
    enable(&pool, w.id).await;
    let d = NaiveDate::from_ymd_opt(2026, 12, 28).unwrap();
    wishly_api::birthday::birthday_tick_at(&pool, d)
        .await
        .unwrap();
    let y: i16 = sqlx::query_scalar("SELECT birthday_notified_year FROM users WHERE id=$1")
        .bind(o.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(y, 2027);
    assert_eq!(queued(&pool, "birthday.friend").await, vec![f.id]);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn nudge_rules(pool: PgPool) {
    let o = user(&pool, "壽星").await;
    set_birthday(&pool, o.id, "10-20", "friends").await;
    // 22 天前不寄；21 天前寄；同年不重寄
    assert_eq!(
        wishly_api::birthday::birthday_tick_at(&pool, day(9, 28))
            .await
            .unwrap(),
        0
    );
    wishly_api::birthday::birthday_tick_at(&pool, day(9, 29))
        .await
        .unwrap();
    wishly_api::birthday::birthday_tick_at(&pool, day(9, 30))
        .await
        .unwrap();
    assert_eq!(
        queued(&pool, "birthday.nudge").await,
        vec![o.id],
        "birthday nudge queued twice"
    );
    // 已開清單者不提醒
    let p = user(&pool, "已開").await;
    set_birthday(&pool, p.id, "10-20", "friends").await;
    let w = wishlist(&pool, p.id, ListOpts::default()).await;
    enable(&pool, w.id).await;
    // 生日 private 者不提醒
    let q = user(&pool, "私").await;
    set_birthday(&pool, q.id, "10-20", "private").await;
    wishly_api::birthday::birthday_tick_at(&pool, day(10, 1))
        .await
        .unwrap();
    assert_eq!(queued(&pool, "birthday.nudge").await, vec![o.id]);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn worker_rechecks_before_sending(pool: PgPool) {
    wishly_api::config::init(wishly_api::config::Config::dev());
    let o = user(&pool, "壽星").await;
    let f = user(&pool, "友").await;
    befriend(&pool, o.id, f.id).await;
    set_birthday(&pool, o.id, "10-05", "friends").await;
    let w = wishlist(&pool, o.id, ListOpts::default()).await;
    enable(&pool, w.id).await;
    wishly_api::birthday::birthday_tick_at(&pool, day(9, 30))
        .await
        .unwrap();
    assert_eq!(queued(&pool, "birthday.friend").await, vec![f.id]);
    sqlx::query("UPDATE wishlists SET visibility='private', birthday_notify=false WHERE id=$1")
        .bind(w.id)
        .execute(&pool)
        .await
        .unwrap();
    wishly_api::notify::tick_at(&pool, common::noon()).await.unwrap();
    let st: String =
        sqlx::query_scalar("SELECT status::text FROM notifications WHERE kind='birthday.friend'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(st, "cancelled");
    // nudge：寄前偏好被關 -> cancelled
    sqlx::query("UPDATE users SET notification_prefs='{\"email_social\": false}' WHERE id=$1")
        .bind(o.id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO notifications(user_id,channel,kind,payload) VALUES($1,'email','birthday.nudge','{}')").bind(o.id).execute(&pool).await.unwrap();
    wishly_api::notify::tick_at(&pool, common::noon()).await.unwrap();
    let st: String =
        sqlx::query_scalar("SELECT status::text FROM notifications WHERE kind='birthday.nudge'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(st, "cancelled");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn unsubscribe_b_token(pool: PgPool) {
    wishly_api::config::init(wishly_api::config::Config::dev());
    let o = user(&pool, "友").await;
    sqlx::query("INSERT INTO notifications(user_id,channel,kind,payload) VALUES($1,'email','birthday.nudge','{}')").bind(o.id).execute(&pool).await.unwrap();
    let t = wishly_api::account::unsub_token('b', o.id);
    let (s, ..) = call(
        &pool,
        "POST",
        "/unsubscribe",
        &[],
        Some(json!({"token": t})),
    )
    .await;
    assert_eq!(s.as_u16(), 204);
    let (p, st): (Value, String) = sqlx::query_as("SELECT (SELECT notification_prefs FROM users WHERE id=$1), status::text FROM notifications WHERE user_id=$1").bind(o.id).fetch_one(&pool).await.unwrap();
    assert_eq!(
        (p["email_social"].clone(), st.as_str()),
        (json!(false), "pending") // pending 不取消，由 worker 寄前靜音
    );
}
