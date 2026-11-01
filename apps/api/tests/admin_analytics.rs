//! GET /admin/analytics：權限、days 驗證、補零、漏斗、排除 staff、k=5 小樣本抑制。
mod common;
use common::*;
use sqlx::PgPool;

#[sqlx::test(migrations = "../../db/migrations")]
async fn cleanup_preserves_login_failures_for_analytics_ranges(pool: PgPool) {
    let s = staff(&pool, "營運").await;
    sqlx::query(
        "INSERT INTO login_failures (email, ip, created_at)
         SELECT 'unknown@example.com', '1.1.1.1',
                (((now() AT TIME ZONE 'Asia/Taipei')::date - age)::timestamp AT TIME ZONE 'Asia/Taipei')
         FROM unnest(ARRAY[0, 2, 20, 60, 364, 366]) age",
    )
    .execute(&pool)
    .await
    .unwrap();
    wishly_api::ratelimit::cleanup(&pool).await.unwrap();
    for (days, expected) in [(7, 2), (30, 3), (90, 4), (365, 5)] {
        let (st, _, b) = call(
            &pool,
            "GET",
            &format!("/admin/analytics?days={days}"),
            &ck(&s.token),
            None,
        )
        .await;
        assert_eq!(st, 200, "{b}");
        assert_eq!(b["trust"]["login_failures"], expected, "days={days}");
    }
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn guards_and_days_validation(pool: PgPool) {
    let u = user(&pool, "一般").await;
    let s = staff(&pool, "營運").await;
    let (st, _, _) = call(&pool, "GET", "/admin/analytics", &ck(&u.token), None).await;
    assert_eq!(st, 403);
    for bad in [6, 366, 0] {
        let (st, _, b) = call(
            &pool,
            "GET",
            &format!("/admin/analytics?days={bad}"),
            &ck(&s.token),
            None,
        )
        .await;
        assert_eq!(st, 422, "{b}");
    }
    let (st, h, b) = call(&pool, "GET", "/admin/analytics?days=7", &ck(&s.token), None).await;
    assert_eq!(st, 200, "{b}");
    assert_eq!(h["cache-control"], "no-store");
    assert_eq!(b["range"]["days"], 7);
    let daily = b["daily"].as_array().unwrap();
    assert_eq!(daily.len(), 7);
    assert_eq!(daily[6]["date"], b["range"]["to"]);
    assert_eq!(daily[0]["date"], b["range"]["from"]);
    // 預設 30 天；staff 不計入
    let (_, _, b) = call(&pool, "GET", "/admin/analytics", &ck(&s.token), None).await;
    assert_eq!(b["daily"].as_array().unwrap().len(), 30);
    assert_eq!(b["totals"]["users"], 1);
    assert_eq!(b["daily"][29]["signups"], 1);
    assert_eq!(b["daily"][0]["signups"], 0);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn funnel_and_staff_excluded(pool: PgPool) {
    let s = staff(&pool, "營運").await;
    let a = user(&pool, "A").await;
    let _idle = user(&pool, "閒置").await;
    let wl = wishlist(&pool, a.id, ListOpts::default()).await;
    let it = item(&pool, wl.id, "杯子", 1).await;
    legacy_guest_claim(&pool, it, 1, None).await;
    // staff 自己的清單不計入
    wishlist(&pool, s.id, ListOpts::default()).await;

    let (_, _, b) = call(&pool, "GET", "/admin/analytics", &ck(&s.token), None).await;
    assert_eq!(b["funnel"]["signed_up"], 2);
    assert_eq!(b["funnel"]["created_list"], 1);
    assert_eq!(b["funnel"]["published_list"], 1);
    assert_eq!(b["funnel"]["received_claim"], 1);
    assert_eq!(b["totals"]["wishlists"], 1);
    assert!(b["totals"].get("guests").is_none());
    assert_eq!(b["totals"]["visitors"], 0);
    assert_eq!(b["claims"]["by_status"]["reserved"], 1);
    assert!(b["claims"].get("by_actor").is_none());
    assert_eq!(b["daily"][29]["claims"], 1);
    assert_eq!(b["daily"][29]["wishlists"], 1);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn small_samples_suppressed(pool: PgPool) {
    let s = staff(&pool, "營運").await;
    let mut ids = vec![];
    for i in 0..6 {
        ids.push(user(&pool, &format!("u{i}")).await.id);
    }
    // 2 人三月、1 人十二月、其餘無生日
    sqlx::query("UPDATE users SET birthday = '03-05' WHERE id = ANY($1)")
        .bind(&ids[..2])
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE users SET birthday = '12-25' WHERE id = $1")
        .bind(ids[2])
        .execute(&pool)
        .await
        .unwrap();
    let (_, _, b) = call(&pool, "GET", "/admin/analytics", &ck(&s.token), None).await;
    assert_eq!(b["birthday_month"].as_object().unwrap().len(), 12);
    assert!(b["birthday_month"]["03"].is_null());
    assert!(b["birthday_month"]["12"].is_null());
    assert_eq!(b["birthday_month"]["01"], 0);
    assert_eq!(b["profile"]["with_birthday"], 3);
    // 6 位非 staff 使用者：locale 桶 ≥5 顯示數字；birthday_visibility 3 人 → null
    assert_eq!(b["distribution"]["locale"]["zh-TW"], 6);
    assert!(b["distribution"]["birthday_visibility"]["friends"].is_null());
    assert!(b["distribution"]["provider"]
        .as_object()
        .unwrap()
        .is_empty());
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn stats_includes_pending_work(pool: PgPool) {
    let st = staff(&pool, "營運").await;
    let u = user(&pool, "申請人").await;
    let o = org(&pool, u.id, "pending").await;
    permit(&pool, o, "pending", 30).await;
    permit(&pool, o, "approved", 30).await;
    let draft = org(&pool, u.id, "draft").await;
    permit(&pool, draft, "pending", 30).await;
    let topup = topup_credit(&pool, u.id, 100).await;
    let wallet: uuid::Uuid = sqlx::query_scalar("SELECT wallet_id FROM topups WHERE id=$1")
        .bind(topup)
        .fetch_one(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE topups SET status='disputed' WHERE id=$1")
        .bind(topup)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO cash_refund_requests(wallet_id,amount,status) VALUES($1,10,'requested'),($1,10,'processing')").bind(wallet).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO wallet_debts(wallet_id,topup_id,amount) VALUES($1,$2,10)")
        .bind(wallet)
        .bind(topup)
        .execute(&pool)
        .await
        .unwrap();
    let w = wishlist(&pool, u.id, ListOpts::default()).await;
    let i = crowdfund_item(&pool, w.id, 100).await;
    sqlx::query("INSERT INTO purchase_orders(item_id,fulfillment_type,amount,shipping_address_snapshot) VALUES($1,'concierge',100,'\\x00')").bind(i).execute(&pool).await.unwrap();
    let (_, _, b) = call(&pool, "GET", "/admin/stats", &ck(&st.token), None).await;
    assert_eq!(b["pending_orgs"], 1);
    assert_eq!(b["pending_permits"], 1);
    for k in [
        "pending_orders",
        "requested_refunds",
        "disputed_topups",
        "open_debts",
    ] {
        assert_eq!(b[k], 1, "{k}");
    }
}
