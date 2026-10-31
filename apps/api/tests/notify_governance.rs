//! 治理 / 組織 / 排程通知的發送點：只在狀態真的改變時寄；event.summary 與 staff.digest 的唯一性與去重。
mod common;
use axum::http::StatusCode;
use common::*;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;
use wishly_api::{notify, notify_jobs};

fn init() {
    let mut c = wishly_api::config::Config::dev();
    c.cf_account_id = None;
    c.cf_email_api_token = None;
    c.smtp_port = 1;
    wishly_api::config::init(c);
}

async fn count(pool: &PgPool, uid: Uuid, kind: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM notifications WHERE user_id=$1 AND kind=$2")
        .bind(uid)
        .bind(kind)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn relief_item(pool: &PgPool, wl: Uuid) -> Uuid {
    sqlx::query_scalar("INSERT INTO wishlist_items(wishlist_id,title,qty_needed,category,urgency) VALUES($1,'礦泉水',10,'water','high') RETURNING id")
        .bind(wl).fetch_one(pool).await.unwrap()
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn moderate_and_suspend_notify_only_on_change(pool: PgPool) {
    let o = user(&pool, "o").await;
    let s = staff(&pool, "s").await;
    let wl = wishlist(&pool, o.id, ListOpts::default()).await;
    let m = |b| async {
        call(
            &pool,
            "PATCH",
            &format!("/admin/wishlists/{}/moderation", wl.id),
            &ck(&s.token),
            Some(b),
        )
        .await
    };
    let hide = json!({"moderation_status": "hidden", "reason": "疑似詐騙"});
    assert_eq!(m(hide.clone()).await.0, StatusCode::OK);
    assert_eq!(m(hide).await.0, StatusCode::OK);
    assert_eq!(
        count(&pool, o.id, "wishlist.moderated").await,
        1,
        "重送不重複"
    );
    assert_eq!(
        m(json!({"moderation_status": "ok"})).await.0,
        StatusCode::OK
    );
    assert_eq!(count(&pool, o.id, "wishlist.moderated").await, 2);

    let u = |b| async {
        call(
            &pool,
            "PATCH",
            &format!("/admin/users/{}", o.id),
            &ck(&s.token),
            Some(b),
        )
        .await
    };
    let block = json!({"blocked": true, "reason": "x"});
    assert_eq!(u(block.clone()).await.0, StatusCode::OK);
    assert_eq!(u(block).await.0, StatusCode::OK);
    assert_eq!(count(&pool, o.id, "account.suspended").await, 1);
    assert_eq!(
        u(json!({"blocked": false, "reason": "x"})).await.0,
        StatusCode::OK
    );
    assert_eq!(count(&pool, o.id, "account.restored").await, 1);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn site_pause_paths_notify_donors_and_owners(pool: PgPool) {
    let s = staff(&pool, "s").await;
    let owner = user(&pool, "owner").await;
    let o = org(&pool, owner.id, "verified").await;
    member(&pool, o, owner.id, "owner").await;
    let p = permit(&pool, o, "pending", 30).await;
    // 許可核准 → org.permit_reviewed；重送 409 不重複
    let (pool_r, tk) = (&pool, s.token.clone());
    let rv = |d: &'static str| {
        let tk = tk.clone();
        async move {
            call(
                pool_r,
                "POST",
                &format!("/admin/permits/{p}/review"),
                &ck(&tk),
                Some(json!({"decision": d, "note": "n"})),
            )
            .await
        }
    };
    assert_eq!(rv("approve").await.0, StatusCode::OK);
    assert_eq!(rv("approve").await.0, StatusCode::CONFLICT);
    assert_eq!(count(&pool, owner.id, "org.permit_reviewed").await, 1);

    let (ev, _) = event(&pool, o).await;
    let w = site(&pool, o, owner.id, ev, p, "active").await;
    let it = relief_item(&pool, w.id).await;
    let (_, _) = user_claim(&pool, it, 1).await;
    // 撤銷已核准許可 → 據點暫停 → 捐助者收 site.paused；不再寄 permit_reviewed
    assert_eq!(rv("reject").await.0, StatusCode::OK);
    let paused: i64 = sqlx::query_scalar("SELECT count(*) FROM notifications WHERE kind='site.paused' AND payload->>'wishlist_id'=$1")
        .bind(w.id.to_string()).fetch_one(&pool).await.unwrap();
    assert_eq!(paused, 1);
    assert_eq!(count(&pool, owner.id, "org.permit_reviewed").await, 1);

    // 事件下架：已暫停的據點不重複通知；另一個仍 active 的據點才通知，且 org.event_hidden 只寄一次
    let p2 = permit(&pool, o, "approved", 30).await;
    let w2 = site(&pool, o, owner.id, ev, p2, "active").await;
    let it2 = relief_item(&pool, w2.id).await;
    user_claim(&pool, it2, 1).await;
    let hide = || async {
        call(
            &pool,
            "PATCH",
            &format!("/admin/relief-events/{ev}"),
            &ck(&s.token),
            Some(json!({"moderation_status": "hidden", "moderation_reason": "x"})),
        )
        .await
    };
    let r = hide().await;
    assert_eq!(r.0, StatusCode::OK, "{}", r.2);
    assert_eq!(hide().await.0, StatusCode::OK);
    let n2: i64 = sqlx::query_scalar("SELECT count(*) FROM notifications WHERE kind='site.paused' AND payload->>'wishlist_id'=$1")
        .bind(w2.id.to_string()).fetch_one(&pool).await.unwrap();
    assert_eq!(n2, 1, "重送不重複");
    assert_eq!(count(&pool, owner.id, "org.event_hidden").await, 1);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn event_summary_sends_once_after_unlock_and_dedups(pool: PgPool) {
    init();
    let owner = user(&pool, "o").await;
    let a = user(&pool, "a").await;
    let b = user(&pool, "b").await;
    // 活動日昨天、未驚喜：兩位會員（其中 a 認領兩件）
    let wl = wishlist(
        &pool,
        owner.id,
        ListOpts {
            event_offset_days: Some(-1),
            ..Default::default()
        },
    )
    .await;
    let i1 = item(&pool, wl.id, "甲", 1).await;
    let i2 = item(&pool, wl.id, "乙", 1).await;
    let i3 = item(&pool, wl.id, "丙", 1).await;
    for (it, u) in [(i1, a.id), (i2, a.id), (i3, b.id)] {
        sqlx::query("INSERT INTO claims(item_id,user_id,status,qty,claimer_name,expires_at) VALUES($1,$2,'purchased',1,'x',now()+interval '1 day')")
            .bind(it).bind(u).execute(&pool).await.unwrap();
    }
    // 驚喜鎖定（活動日在未來）與 0 人的清單都不寄
    let locked = wishlist(
        &pool,
        owner.id,
        ListOpts {
            surprise: true,
            event_offset_days: Some(3),
            ..Default::default()
        },
    )
    .await;
    let empty = wishlist(
        &pool,
        owner.id,
        ListOpts {
            event_offset_days: Some(-1),
            ..Default::default()
        },
    )
    .await;
    assert_eq!(notify_jobs::event_summary_tick(&pool).await.unwrap(), 1);
    assert_eq!(
        notify_jobs::event_summary_tick(&pool).await.unwrap(),
        0,
        "只寄一次"
    );
    let givers: i64 = sqlx::query_scalar("SELECT (payload->>'givers')::bigint FROM notifications WHERE kind='event.summary' AND payload->>'wishlist_id'=$1")
        .bind(wl.id.to_string()).fetch_one(&pool).await.unwrap();
    assert_eq!(givers, 2, "人數去重");
    for w in [locked.id, empty.id] {
        let sent: bool = sqlx::query_scalar(
            "SELECT event_summary_sent_at IS NOT NULL FROM wishlists WHERE id=$1",
        )
        .bind(w)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(!sent, "未解鎖 / 0 人不能用掉 event_summary_sent_at");
    }
    // 解鎖（活動日改為昨天）後才寄
    sqlx::query("UPDATE wishlists SET event_date = current_date - 1 WHERE id=$1")
        .bind(locked.id)
        .execute(&pool)
        .await
        .unwrap();
    let it = item(&pool, locked.id, "丁", 1).await;
    sqlx::query("INSERT INTO claims(item_id,user_id,status,qty,claimer_name,expires_at) VALUES($1,$2,'delivered',1,'x',now()+interval '1 day')")
        .bind(it).bind(a.id).execute(&pool).await.unwrap();
    assert_eq!(notify_jobs::event_summary_tick(&pool).await.unwrap(), 1);
    // worker 在鎖定中取消 event.summary（鎖定期間補塞的列）
    sqlx::query("UPDATE wishlists SET event_date = current_date + 5 WHERE id=$1")
        .bind(locked.id)
        .execute(&pool)
        .await
        .unwrap();
    notify::tick_at(&pool, noon()).await.unwrap();
    let st: String = sqlx::query_scalar("SELECT status::text FROM notifications WHERE kind='event.summary' AND payload->>'wishlist_id'=$1")
        .bind(locked.id.to_string()).fetch_one(&pool).await.unwrap();
    assert_eq!(st, "cancelled");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn staff_digest_only_with_work_and_once_per_20h(pool: PgPool) {
    let s1 = staff(&pool, "s1").await;
    let s2 = staff(&pool, "s2").await;
    sqlx::query("UPDATE users SET blocked_at=now() WHERE id=$1")
        .bind(s2.id)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        notify_jobs::staff_digest_tick(&pool).await.unwrap(),
        0,
        "沒有待辦不寄"
    );
    let o = user(&pool, "o").await;
    let wl = wishlist(&pool, o.id, ListOpts::default()).await;
    sqlx::query("INSERT INTO content_reports(wishlist_id,reason) VALUES($1,'scam')")
        .bind(wl.id)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        notify_jobs::staff_digest_tick(&pool).await.unwrap(),
        1,
        "封鎖的 staff 不寄"
    );
    assert_eq!(
        notify_jobs::staff_digest_tick(&pool).await.unwrap(),
        0,
        "20 小時內不重寄"
    );
    assert_eq!(count(&pool, s1.id, "staff.digest").await, 1);
    let p: serde_json::Value =
        sqlx::query_scalar("SELECT payload FROM notifications WHERE kind='staff.digest'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(p["reports"], 1);
}
