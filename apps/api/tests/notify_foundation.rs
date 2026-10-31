//! 通知基礎設施：worker 順序（安靜時段 / 驚喜把關 / 合併 / 偏好 / 退避）、站內通知 API、requeue、退訂與偏好。
//! 寄信一律指向不存在的 SMTP 埠：能寄成功的路徑由 smtp_worker.rs 覆蓋，這裡要驗的是「寄不出去」與「根本不寄」。
mod common;
use axum::http::StatusCode;
use chrono::{TimeZone, Utc};
use common::*;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;
use wishly_api::notify;

fn init() {
    let mut c = wishly_api::config::Config::dev();
    c.cf_account_id = None;
    c.cf_email_api_token = None;
    c.smtp_port = 1;
    wishly_api::config::init(c);
}

async fn push(pool: &PgPool, uid: Uuid, kind: &str, payload: Value) -> Uuid {
    sqlx::query_scalar("INSERT INTO notifications (user_id, channel, kind, payload) VALUES ($1,'email',$2,$3) RETURNING id")
        .bind(uid).bind(kind).bind(payload).fetch_one(pool).await.unwrap()
}

type Row = (String, Option<String>, i16, bool, Option<String>);
async fn row(pool: &PgPool, id: Uuid) -> Row {
    sqlx::query_as("SELECT status::text, last_error, attempts, inbox_at IS NOT NULL, inbox_title FROM notifications WHERE id=$1")
        .bind(id).fetch_one(pool).await.unwrap()
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn quiet_hours_defer_gifts_but_not_account(pool: PgPool) {
    init();
    let u = user(&pool, "u").await;
    let wl = wishlist(&pool, u.id, ListOpts::default()).await;
    let gifts = push(&pool, u.id, "site.paused", json!({"wishlist_id": wl.id})).await;
    let social = push(&pool, u.id, "birthday.nudge", json!({})).await;
    let closing = push(&pool, u.id, "wishlist.closing", json!({"wishlist_id": wl.id})).await;
    let account = push(&pool, u.id, "wallet.unfrozen", json!({})).await;
    // 台北 23:00（UTC 15:00）
    let now = Utc.with_ymd_and_hms(2026, 10, 31, 15, 0, 0).unwrap();
    notify::tick_at(&pool, now).await.unwrap();
    for id in [gifts, social] {
        let (st, _, attempts, inbox, _) = row(&pool, id).await;
        assert_eq!((st.as_str(), attempts, inbox), ("pending", 0, false));
    }
    let at: chrono::DateTime<Utc> = sqlx::query_scalar("SELECT scheduled_at FROM notifications WHERE id=$1").bind(gifts).fetch_one(&pool).await.unwrap();
    assert_eq!(at, Utc.with_ymd_and_hms(2026, 11, 1, 1, 0, 0).unwrap(), "延到下一個台北 09:00");
    // wishlist.closing 與 Account 不延後（寄不出去所以留在 pending，但已嘗試並寫入站內信）
    for id in [closing, account] {
        let (_, _, attempts, inbox, _) = row(&pool, id).await;
        assert_eq!((attempts, inbox), (1, true));
    }
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn surprise_lock_cancels_owner_side_kinds(pool: PgPool) {
    init();
    let u = user(&pool, "u").await;
    let locked = wishlist(&pool, u.id, ListOpts { surprise: true, event_offset_days: Some(30), ..Default::default() }).await;
    let open = wishlist(&pool, u.id, ListOpts::default()).await;
    let it = item(&pool, locked.id, "禮物", 1).await;
    let mut ids = vec![];
    for k in ["owner.funded", "owner.shipped", "claim.cancelled_by_donor", "item.reopened", "event.summary"] {
        ids.push(push(&pool, u.id, k, json!({"wishlist_id": locked.id, "item_id": it, "givers": 2})).await);
    }
    notify::tick_at(&pool, noon()).await.unwrap();
    for id in ids {
        let (st, err, _, inbox, _) = row(&pool, id).await;
        assert_eq!((st.as_str(), err.as_deref(), inbox), ("cancelled", Some("surprise locked"), false));
    }
    // 未鎖定的清單照常處理（進站內信）；wishlist.moderated 不在把關名單，鎖定中也送達
    let ok = push(&pool, u.id, "owner.funded", json!({"wishlist_id": open.id})).await;
    let moderated = push(&pool, u.id, "wishlist.moderated", json!({"wishlist_id": locked.id, "hidden": true, "reason": "x"})).await;
    notify::tick_at(&pool, noon()).await.unwrap();
    assert!(row(&pool, ok).await.3 && row(&pool, moderated).await.3);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn bundles_same_kind_into_one_mail(pool: PgPool) {
    init();
    let owner = user(&pool, "o").await;
    let u = user(&pool, "u").await;
    let wl = wishlist(&pool, owner.id, ListOpts::default()).await;
    let mut ids = vec![];
    for t in ["甲", "乙", "丙"] {
        let it = item(&pool, wl.id, t, 1).await;
        ids.push(push(&pool, u.id, "claim.expired", json!({"wishlist_id": wl.id, "item_id": it})).await);
    }
    // 另一位收件人不受影響
    let other = push(&pool, owner.id, "claim.expired", json!({"wishlist_id": wl.id})).await;
    notify::tick_at(&pool, noon()).await.unwrap();
    let mut main = 0;
    let mut bundled = 0;
    for id in &ids {
        let (st, err, _, inbox, title) = row(&pool, *id).await;
        if st == "sent" && err.as_deref().is_some_and(|e| e.starts_with("bundled:")) {
            bundled += 1;
            assert!(!inbox);
        } else {
            main += 1;
            assert!(inbox && title.unwrap().ends_with("（共 3 件）"));
        }
    }
    assert_eq!((main, bundled), (1, 2));
    assert!(row(&pool, other).await.3);
    // 內文列出其餘品項
    let (_, body) = notify::render_lang(notify::Lang::Zh, "claim.expired", "清單", &json!({"item_title": "甲", "more_items": ["乙", "丙"]}));
    assert!(body.contains("- 乙") && body.contains("- 丙"));
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn stale_confirmation_is_cancelled_and_not_bundled(pool: PgPool) {
    init();
    let owner = user(&pool, "o").await;
    let wl = wishlist(&pool, owner.id, ListOpts::default()).await;
    let it = item(&pool, wl.id, "禮物", 5).await;
    let (_, live) = user_claim(&pool, it, 1).await;
    let (_, gone) = user_claim(&pool, it, 1).await;
    sqlx::query("UPDATE claims SET status='cancelled' WHERE id=$1").bind(gone).execute(&pool).await.unwrap();
    let uid: Uuid = sqlx::query_scalar("SELECT user_id FROM claims WHERE id=$1").bind(live).fetch_one(&pool).await.unwrap();
    let a = push(&pool, uid, "claim.confirmation", json!({"wishlist_id": wl.id, "item_id": it, "claim_id": live})).await;
    let b = push(&pool, uid, "claim.confirmation", json!({"wishlist_id": wl.id, "item_id": it, "claim_id": gone})).await;
    notify::tick_at(&pool, noon()).await.unwrap();
    assert_eq!(row(&pool, b).await.1.as_deref(), Some("claim no longer active"));
    assert_eq!(row(&pool, a).await.2, 1);
    assert!(sqlx::query_scalar::<_, Option<String>>("SELECT payload->>'more_items' FROM notifications WHERE id=$1").bind(a).fetch_one(&pool).await.unwrap().is_none());
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn failed_send_backs_off_then_fails(pool: PgPool) {
    init();
    let u = user(&pool, "u").await;
    let id = push(&pool, u.id, "wallet.unfrozen", json!({})).await;
    notify::tick(&pool).await.unwrap();
    let (st, err, attempts, ..) = row(&pool, id).await;
    assert_eq!((st.as_str(), attempts), ("pending", 1));
    assert!(err.is_some());
    let secs: f64 = sqlx::query_scalar("SELECT extract(epoch FROM scheduled_at - now())::float8 FROM notifications WHERE id=$1").bind(id).fetch_one(&pool).await.unwrap();
    assert!((50.0..=61.0).contains(&secs), "第一次退避 1 分鐘，實際 {secs}");
    // 領取租約：還沒到期不會被撿起
    assert_eq!(notify::tick(&pool).await.unwrap(), 0);
    // 第 6 次仍失敗 -> failed
    sqlx::query("UPDATE notifications SET attempts = 5, scheduled_at = now() WHERE id=$1").bind(id).execute(&pool).await.unwrap();
    notify::tick(&pool).await.unwrap();
    let (st, _, attempts, ..) = row(&pool, id).await;
    assert_eq!((st.as_str(), attempts), ("failed", 6));
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn muted_and_emailless_users_still_get_inbox(pool: PgPool) {
    init();
    let u = user(&pool, "u").await;
    let wl = wishlist(&pool, u.id, ListOpts::default()).await;
    sqlx::query("UPDATE users SET notification_prefs='{\"email_gifts\": false, \"email_social\": false}' WHERE id=$1").bind(u.id).execute(&pool).await.unwrap();
    let gifts = push(&pool, u.id, "site.paused", json!({"wishlist_id": wl.id})).await;
    let from = user(&pool, "阿花").await;
    let social = push(&pool, u.id, "friend.accepted", json!({"from_user": from.id})).await;
    let account = push(&pool, u.id, "wallet.frozen", json!({})).await;
    notify::tick_at(&pool, noon()).await.unwrap();
    for id in [gifts, social] {
        let (st, err, _, inbox, _) = row(&pool, id).await;
        assert_eq!((st.as_str(), err.as_deref(), inbox), ("cancelled", Some("muted"), true));
    }
    let (st, _, _, inbox, _) = row(&pool, account).await;
    assert_eq!((st.as_str(), inbox), ("pending", true), "Account 不可關閉：照常嘗試寄送");
    let t: String = sqlx::query_scalar("SELECT inbox_title FROM notifications WHERE id=$1").bind(social).fetch_one(&pool).await.unwrap();
    assert_eq!(t, "阿花 接受了你的好友邀請");
    // 沒有 email 的使用者
    let n = user(&pool, "n").await;
    sqlx::query("UPDATE users SET email=NULL WHERE id=$1").bind(n.id).execute(&pool).await.unwrap();
    let id = push(&pool, n.id, "site.paused", json!({"wishlist_id": wl.id})).await;
    notify::tick_at(&pool, noon()).await.unwrap();
    let (st, err, _, inbox, _) = row(&pool, id).await;
    assert_eq!((st.as_str(), err.as_deref(), inbox), ("cancelled", Some("no email recipient"), true));
    let link: String = sqlx::query_scalar("SELECT inbox_link FROM notifications WHERE id=$1").bind(id).fetch_one(&pool).await.unwrap();
    assert_eq!(link, "/me/claims");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn inbox_api_is_scoped_to_caller(pool: PgPool) {
    let a = user(&pool, "a").await;
    let b = user(&pool, "b").await;
    let mut a_ids = vec![];
    for i in 0..5 {
        let id = push(&pool, a.id, "wallet.frozen", json!({})).await;
        sqlx::query("UPDATE notifications SET inbox_at = now() - make_interval(mins => $2), inbox_title = $3, inbox_link = '/me/wallet' WHERE id=$1")
            .bind(id).bind(i).bind(format!("t{i}")).execute(&pool).await.unwrap();
        a_ids.push(id);
    }
    let bid = push(&pool, b.id, "wallet.frozen", json!({})).await;
    sqlx::query("UPDATE notifications SET inbox_at = now(), inbox_title = 'b-only' WHERE id=$1").bind(bid).execute(&pool).await.unwrap();
    push(&pool, a.id, "wallet.frozen", json!({})).await; // 未進站內信
    let (s, _, v) = call(&pool, "GET", "/me/notifications?limit=2", &ck(&a.token), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["unread"], 5);
    let titles: Vec<&str> = v["items"].as_array().unwrap().iter().map(|i| i["title"].as_str().unwrap()).collect();
    assert_eq!(titles, ["t0", "t1"]);
    let cursor = v["next_cursor"].as_str().unwrap().to_string();
    let (_, _, v2) = call(&pool, "GET", &format!("/me/notifications?limit=10&before={cursor}"), &ck(&a.token), None).await;
    let titles: Vec<&str> = v2["items"].as_array().unwrap().iter().map(|i| i["title"].as_str().unwrap()).collect();
    assert_eq!((titles, v2["next_cursor"].is_null()), (vec!["t2", "t3", "t4"], true));
    assert_eq!(call(&pool, "GET", "/me/notifications?limit=0", &ck(&a.token), None).await.0, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(call(&pool, "GET", "/me/notifications", &[], None).await.0, StatusCode::UNAUTHORIZED);
    // 非本人 404；本人標已讀
    assert_eq!(call(&pool, "POST", &format!("/me/notifications/{bid}/read"), &ck(&a.token), None).await.0, StatusCode::NOT_FOUND);
    assert_eq!(call(&pool, "POST", &format!("/me/notifications/{}/read", a_ids[0]), &ck(&a.token), None).await.0, StatusCode::NO_CONTENT);
    let (_, _, c) = call(&pool, "GET", "/me/notifications/unread-count", &ck(&a.token), None).await;
    assert_eq!(c["unread"], 4);
    assert_eq!(call(&pool, "POST", "/me/notifications/read-all", &ck(&a.token), None).await.0, StatusCode::NO_CONTENT);
    let (_, _, c) = call(&pool, "GET", "/me/notifications/unread-count", &ck(&a.token), None).await;
    assert_eq!(c["unread"], 0);
    let (_, _, cb) = call(&pool, "GET", "/me/notifications/unread-count", &ck(&b.token), None).await;
    assert_eq!(cb["unread"], 1, "read-all 不影響他人");
    // 匯出含站內通知標題
    let (_, _, ex) = call(&pool, "GET", "/me/export", &ck(&a.token), None).await;
    assert_eq!(ex["notifications"].as_array().unwrap().len(), 5);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn account_deletion_clears_inbox(pool: PgPool) {
    let _cdn = common::cdn::init();
    let a = user(&pool, "a").await;
    let id = push(&pool, a.id, "wallet.frozen", json!({})).await;
    sqlx::query("UPDATE notifications SET inbox_at = now(), inbox_title = 'x', inbox_link = '/y' WHERE id=$1").bind(id).execute(&pool).await.unwrap();
    let (s, ..) = call(&pool, "DELETE", "/me", &ck(&a.token), Some(json!({"confirm": "DELETE"}))).await;
    assert_eq!(s, StatusCode::OK);
    let r: (bool, Option<String>, Option<String>) = sqlx::query_as("SELECT inbox_at IS NULL, inbox_title, inbox_link FROM notifications WHERE id=$1").bind(id).fetch_one(&pool).await.unwrap();
    assert_eq!(r, (true, None, None));
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn requeue_only_recent_non_address_failures(pool: PgPool) {
    let st = staff(&pool, "s").await;
    let u = user(&pool, "u").await;
    let mk = |err: &'static str, age: &'static str| {
        let pool = pool.clone();
        async move {
            let id = push(&pool, u.id, "wallet.frozen", json!({})).await;
            sqlx::query("UPDATE notifications SET status='failed', attempts=6, last_error=$2, scheduled_at = now() - $3::interval WHERE id=$1")
                .bind(id).bind(err).bind(age).execute(&pool).await.unwrap();
            id
        }
    };
    let recent = mk("smtp down", "1 hour").await;
    let bad = mk("bad address", "1 hour").await;
    let old = mk("smtp down", "10 days").await;
    assert_eq!(call(&pool, "POST", "/admin/notifications/requeue", &ck(&u.token), None).await.0, StatusCode::FORBIDDEN);
    let (s, _, v) = call(&pool, "POST", "/admin/notifications/requeue", &ck(&st.token), None).await;
    assert_eq!((s, &v["requeued"]), (StatusCode::OK, &json!(1)));
    let r: (String, i16) = sqlx::query_as("SELECT status::text, attempts FROM notifications WHERE id=$1").bind(recent).fetch_one(&pool).await.unwrap();
    assert_eq!(r, ("pending".into(), 0));
    assert_eq!(row(&pool, bad).await.0, "failed");
    assert_eq!(row(&pool, old).await.0, "failed");
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE action='notification.requeue'").fetch_one(&pool).await.unwrap();
    assert_eq!(n, 1);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn prefs_and_unsubscribe_tokens(pool: PgPool) {
    let u = user(&pool, "u").await;
    for bad in [json!({"email_claims": false}), json!({"email_birthdays": false}), json!({"email_gifts": "no"})] {
        let (s, ..) = call(&pool, "PATCH", "/me", &ck(&u.token), Some(json!({"notification_prefs": bad}))).await;
        assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{bad}");
    }
    let (s, _, me) = call(&pool, "PATCH", "/me", &ck(&u.token), Some(json!({"notification_prefs": {"email_social": false}}))).await;
    assert_eq!((s, &me["notification_prefs"]), (StatusCode::OK, &json!({"email_social": false})));
    // 'c' 關送禮進度、'b' 關社交；舊的 'u' 不再有效
    let post = |t: String| call(&pool, "POST", "/unsubscribe", &[], Some(json!({"token": t})));
    assert_eq!(post(wishly_api::account::unsub_token('c', u.id)).await.0, StatusCode::NO_CONTENT);
    assert_eq!(post(wishly_api::account::unsub_token('u', u.id)).await.0, StatusCode::UNPROCESSABLE_ENTITY);
    let p: Value = sqlx::query_scalar("SELECT notification_prefs FROM users WHERE id=$1").bind(u.id).fetch_one(&pool).await.unwrap();
    assert_eq!(p, json!({"email_social": false, "email_gifts": false}));
    // 舊鍵遷移後的預設是空物件
    let n = user(&pool, "n").await;
    let d: Value = sqlx::query_scalar("SELECT notification_prefs FROM users WHERE id=$1").bind(n.id).fetch_one(&pool).await.unwrap();
    assert_eq!(d, json!({}));
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn enqueue_helpers(pool: PgPool) {
    let owner = user(&pool, "o").await;
    let wl = wishlist(&pool, owner.id, ListOpts::default()).await;
    let it = item(&pool, wl.id, "禮物", 3).await;
    let (_, claim) = user_claim(&pool, it, 1).await;
    // 會員認領確認：延後約 10 分鐘
    notify::enqueue_claim_confirmation(&pool, claim).await.unwrap();
    let (secs, p): (f64, Value) = sqlx::query_as("SELECT extract(epoch FROM scheduled_at - now())::float8, payload FROM notifications WHERE kind='claim.confirmation'").fetch_one(&pool).await.unwrap();
    assert!((590.0..=601.0).contains(&secs));
    assert_eq!(p["claim_id"], json!(claim));
    // 個人清單 -> owner_id；payload 併入 wishlist_id
    notify::enqueue_owner(&pool, wl.id, "owner.funded", json!({"item_id": it})).await.unwrap();
    let (to, p): (Uuid, Value) = sqlx::query_as("SELECT user_id, payload FROM notifications WHERE kind='owner.funded'").fetch_one(&pool).await.unwrap();
    assert_eq!((to, &p["wishlist_id"]), (owner.id, &json!(wl.id)));
    // 組織清單 -> 未停用的 owner / coordinator，不含 volunteer 與已停用者
    let o = org(&pool, owner.id, "verified").await;
    let (co, vol, off) = (user(&pool, "co").await, user(&pool, "v").await, user(&pool, "off").await);
    member(&pool, o, owner.id, "owner").await;
    member(&pool, o, co.id, "coordinator").await;
    member(&pool, o, vol.id, "volunteer").await;
    member(&pool, o, off.id, "coordinator").await;
    sqlx::query("UPDATE org_members SET disabled_at=now() WHERE user_id=$1").bind(off.id).execute(&pool).await.unwrap();
    let pm = permit(&pool, o, "approved", 30).await;
    let (ev, _) = event(&pool, o).await;
    let site = site(&pool, o, owner.id, ev, pm, "active").await;
    notify::enqueue_owner(&pool, site.id, "owner.shipped", json!({})).await.unwrap();
    let mut got: Vec<Uuid> = sqlx::query_scalar("SELECT user_id FROM notifications WHERE kind='owner.shipped'").fetch_all(&pool).await.unwrap();
    got.sort();
    let mut want = vec![owner.id, co.id];
    want.sort();
    assert_eq!(got, want);
    notify::enqueue_org_owners(&pool, o, "org.permit_reviewed", json!({"org_id": o})).await.unwrap();
    let got: Vec<Uuid> = sqlx::query_scalar("SELECT user_id FROM notifications WHERE kind='org.permit_reviewed'").fetch_all(&pool).await.unwrap();
    assert_eq!(got, vec![owner.id]);
    // 據點暫停：認領者每人一筆
    let sit: Uuid = sqlx::query_scalar("INSERT INTO wishlist_items(wishlist_id,title,qty_needed,category,urgency) VALUES($1,'物資',5,'water','high') RETURNING id")
        .bind(site.id).fetch_one(&pool).await.unwrap();
    let (_, c1) = user_claim(&pool, sit, 1).await;
    let (_, c2) = user_claim(&pool, sit, 1).await;
    notify::enqueue_site_paused(&pool, &[site.id]).await.unwrap();
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM notifications WHERE kind='site.paused'").fetch_one(&pool).await.unwrap();
    let _ = (c1, c2);
    assert_eq!(n, 2);
}
