//! 通知發送點：認領確認 / 取消、好友接受。
mod common;
use common::*;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;
use wishly_api::notify;

async fn count(pool: &PgPool, kind: &str, user: Option<Uuid>) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM notifications WHERE kind = $1 AND ($2::uuid IS NULL OR user_id = $2)")
        .bind(kind).bind(user).fetch_one(pool).await.unwrap()
}

async fn patch(pool: &PgPool, tok: &str, claim: Uuid, status: &str) {
    let (s, _, b) = call(pool, "PATCH", &format!("/claims/{claim}"), &ck(tok), Some(json!({"status": status}))).await;
    assert_eq!(s, axum::http::StatusCode::OK, "{b}");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn create_enqueues_delayed_confirmation(pool: PgPool) {
    let o = user(&pool, "o").await;
    let wl = wishlist(&pool, o.id, ListOpts::default()).await;
    let it = item(&pool, wl.id, "奶瓶", 3).await;
    let d = user(&pool, "d").await;
    let (s, _, b) = call(&pool, "POST", &format!("/items/{it}/claims"), &[ck(&d.token), idem(Uuid::new_v4())].concat(), Some(json!({"qty": 1}))).await;
    assert_eq!(s, axum::http::StatusCode::CREATED, "{b}");
    let delayed: bool = sqlx::query_scalar("SELECT scheduled_at > now() + interval '5 minutes' FROM notifications WHERE kind='claim.confirmation' AND user_id=$1")
        .bind(d.id).fetch_one(&pool).await.unwrap();
    assert!(delayed);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn owner_cancel_notifies_donor_and_legacy_guest(pool: PgPool) {
    let o = user(&pool, "o").await;
    let wl = wishlist(&pool, o.id, ListOpts::default()).await;
    let it = item(&pool, wl.id, "奶瓶", 3).await;
    let (dtok, c) = user_claim(&pool, it, 1).await;
    let _ = dtok;
    patch(&pool, &o.token, c, "cancelled").await;
    assert_eq!(count(&pool, "claim.cancelled_by_owner", None).await, 1);
    assert_eq!(count(&pool, "claim.cancelled_by_donor", None).await, 0);

    let (_, gc) = legacy_guest_claim(&pool, it, 1, Some("g@example.com")).await;
    patch(&pool, &o.token, gc, "cancelled").await;
    let g: i64 = sqlx::query_scalar("SELECT count(*) FROM notifications WHERE kind='claim.cancelled_by_owner' AND guest_id IS NOT NULL")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(g, 1);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn donor_cancel_notifies_owner_only_from_purchased(pool: PgPool) {
    let o = user(&pool, "o").await;
    let wl = wishlist(&pool, o.id, ListOpts::default()).await;
    let it = item(&pool, wl.id, "奶瓶", 3).await;
    let (t1, c1) = user_claim(&pool, it, 1).await;
    patch(&pool, &t1, c1, "cancelled").await; // reserved → cancelled
    assert_eq!(count(&pool, "claim.cancelled_by_donor", None).await, 0);
    let (t2, c2) = user_claim(&pool, it, 1).await;
    patch(&pool, &t2, c2, "purchased").await;
    patch(&pool, &t2, c2, "cancelled").await;
    assert_eq!(count(&pool, "claim.cancelled_by_donor", Some(o.id)).await, 1);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn donor_cancel_not_sent_while_surprise_locked(pool: PgPool) {
    let o = user(&pool, "o").await;
    let wl = wishlist(&pool, o.id, ListOpts::default()).await;
    sqlx::query("UPDATE wishlists SET surprise_mode = true, event_date = current_date + 30 WHERE id = $1")
        .bind(wl.id).execute(&pool).await.unwrap();
    let it = item(&pool, wl.id, "奶瓶", 3).await;
    let (t, c) = user_claim(&pool, it, 1).await;
    patch(&pool, &t, c, "purchased").await;
    patch(&pool, &t, c, "cancelled").await;
    assert_eq!(count(&pool, "claim.cancelled_by_donor", Some(o.id)).await, 1);
    notify::tick_at(&pool, noon()).await.unwrap();
    let st: String = sqlx::query_scalar("SELECT status::text FROM notifications WHERE kind='claim.cancelled_by_donor'")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(st, "cancelled");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn friend_accept_notifies_other_side(pool: PgPool) {
    let a = user(&pool, "a").await;
    let b = user(&pool, "b").await;
    let (s, _, r) = call(&pool, "POST", "/friends/requests", &ck(&a.token), Some(json!({"to": b.id.to_string()}))).await;
    assert!(s.is_success(), "{r}");
    let rid: Uuid = sqlx::query_scalar("SELECT id FROM friend_requests WHERE from_user=$1").bind(a.id).fetch_one(&pool).await.unwrap();
    let (s, _, r) = call(&pool, "POST", &format!("/friends/requests/{rid}/accept"), &ck(&b.token), None).await;
    assert!(s.is_success(), "{r}");
    let p: serde_json::Value = sqlx::query_scalar("SELECT payload FROM notifications WHERE kind='friend.accepted' AND user_id=$1")
        .bind(a.id).fetch_one(&pool).await.unwrap();
    assert_eq!(p["from_user"], json!(b.id));

    // 邀請連結：接受方通知邀請人
    let (s, _, inv) = call(&pool, "POST", "/friends/invites", &ck(&a.token), None).await;
    assert!(s.is_success(), "{inv}");
    let tok = inv["token"].as_str().unwrap();
    let c = user(&pool, "c").await;
    let (s, _, r) = call(&pool, "POST", &format!("/friends/invites/{tok}/accept"), &ck(&c.token), None).await;
    assert!(s.is_success(), "{r}");
    assert_eq!(count(&pool, "friend.accepted", Some(a.id)).await, 2);
}
