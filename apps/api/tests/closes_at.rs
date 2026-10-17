//! 清單自動結束時間 closes_at：建立 / 驗證、到期掃描、到期前提醒、捐助與認捐即時擋、公開頁。
mod common;
use axum::http::StatusCode;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;
use wishly_api::wishlists;

use common::ck;

/// 清單；closes_sql 是 SQL 運算式（如 "now() + interval '1 hour'" 或 "NULL"）
async fn deadline_list(pool: &PgPool, owner: Uuid, status: &'static str, closes_sql: &str) -> Uuid {
    let list = common::wishlist(
        pool,
        owner,
        common::ListOpts {
            status: Some(status),
            ..Default::default()
        },
    )
    .await;
    sqlx::query(&format!(
        "UPDATE wishlists SET closes_at={closes_sql} WHERE id=$1"
    ))
    .bind(list.id)
    .execute(pool)
    .await
    .unwrap();
    list.id
}
async fn status_of(pool: &PgPool, id: Uuid) -> String {
    sqlx::query_scalar("SELECT status::text FROM wishlists WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}
async fn closing_notifs(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM notifications WHERE kind = 'wishlist.closing'")
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn create_with_and_without_closes_at(pool: PgPool) {
    let _cdn = common::cdn::init();
    let common::TestUser { id: _, token: t } = common::user(&pool, "u").await;
    let (s, _, b) = common::call(
        &pool,
        "POST",
        "/wishlists",
        &ck(&t),
        Some(json!({ "type": "personal", "title": "不設期限" })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    assert!(b["closes_at"].is_null(), "預設不設期限");

    let (s, _, b) = common::call(
        &pool,
        "POST",
        "/wishlists",
        &ck(&t),
        Some(json!({ "type": "personal", "title": "有期限", "closes_at": "2999-01-01T00:00:00Z" })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    assert_eq!(b["closes_at"], "2999-01-01T00:00:00Z");

    for bad in [
        json!("2000-01-01T00:00:00Z"),
        json!("not-a-time"),
        json!(123),
    ] {
        let (s, _, b) = common::call(
            &pool,
            "POST",
            "/wishlists",
            &ck(&t),
            Some(json!({ "type": "personal", "title": "壞", "closes_at": bad })),
        )
        .await;
        assert_eq!(
            (s.as_u16(), b["errors"][0]["pointer"].as_str()),
            (422, Some("/closes_at")),
            "{bad}: {b}"
        );
    }
    common::assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn patch_sets_clears_and_keeps_closes_at(pool: PgPool) {
    let _cdn = common::cdn::init();
    let common::TestUser { id: o, token: t } = common::user(&pool, "u").await;
    let wid = deadline_list(&pool, o, "active", "NULL").await;
    let uri = format!("/wishlists/{wid}");
    let (_, _, b) = common::call(
        &pool,
        "PATCH",
        &uri,
        &ck(&t),
        Some(json!({ "closes_at": "2999-01-01T00:00:00Z" })),
    )
    .await;
    assert_eq!(b["closes_at"], "2999-01-01T00:00:00Z");
    // 不帶欄位 = 不動
    let (_, _, b) = common::call(
        &pool,
        "PATCH",
        &uri,
        &ck(&t),
        Some(json!({ "title": "改名" })),
    )
    .await;
    assert_eq!(b["closes_at"], "2999-01-01T00:00:00Z");
    // null = 清除
    let (_, _, b) = common::call(
        &pool,
        "PATCH",
        &uri,
        &ck(&t),
        Some(json!({ "closes_at": null })),
    )
    .await;
    assert!(b["closes_at"].is_null());
    common::assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn close_due_only_touches_active_and_expired(pool: PgPool) {
    let _cdn = common::cdn::init();
    let common::TestUser { id: o, token: _ } = common::user(&pool, "u").await;
    let due = deadline_list(&pool, o, "active", "now() - interval '1 minute'").await;
    let future = deadline_list(&pool, o, "active", "now() + interval '1 hour'").await;
    let none = deadline_list(&pool, o, "active", "NULL").await;
    let draft = deadline_list(&pool, o, "draft", "now() - interval '1 minute'").await;
    assert_eq!(wishlists::close_due(&pool).await.unwrap(), 1);
    assert_eq!(status_of(&pool, due).await, "closed");
    let closed_at: Option<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar("SELECT closed_at FROM wishlists WHERE id = $1")
            .bind(due)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(closed_at.is_some());
    for (id, st) in [(future, "active"), (none, "active"), (draft, "draft")] {
        assert_eq!(status_of(&pool, id).await, st);
    }
    assert_eq!(wishlists::close_due(&pool).await.unwrap(), 0, "冪等");
    common::assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn reminder_sent_once_and_rearmed_on_change(pool: PgPool) {
    let _cdn = common::cdn::init();
    let common::TestUser { id: o, token: t } = common::user(&pool, "u").await;
    let soon = deadline_list(&pool, o, "active", "now() + interval '2 hours'").await;
    let far = deadline_list(&pool, o, "active", "now() + interval '3 days'").await;
    let _none = deadline_list(&pool, o, "active", "NULL").await;
    let _closed = deadline_list(&pool, o, "closed", "now() + interval '2 hours'").await;
    let _past = deadline_list(&pool, o, "active", "now() - interval '1 hour'").await;

    assert_eq!(
        wishlists::remind_closing(&pool).await.unwrap(),
        1,
        "只有 24 小時內、active、未過期者"
    );
    assert_eq!(
        wishlists::remind_closing(&pool).await.unwrap(),
        0,
        "只提醒一次"
    );
    assert_eq!(closing_notifs(&pool).await, 1);
    let (uid, wl): (Uuid, String) = sqlx::query_as("SELECT user_id, payload->>'wishlist_id' FROM notifications WHERE kind = 'wishlist.closing'").fetch_one(&pool).await.unwrap();
    assert_eq!((uid, wl), (o, soon.to_string()));

    // 創建者改期 → 重新武裝；遠期不提醒
    let (s, _, _) = common::call(
        &pool,
        "PATCH",
        &format!("/wishlists/{soon}"),
        &ck(&t),
        Some(
            json!({ "closes_at": (chrono::Utc::now() + chrono::Duration::hours(5)).to_rfc3339() }),
        ),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(wishlists::remind_closing(&pool).await.unwrap(), 1);
    // 沒改動的 PATCH 不會重置
    let (s, _, _) = common::call(
        &pool,
        "PATCH",
        &format!("/wishlists/{soon}"),
        &ck(&t),
        Some(json!({ "title": "x" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(wishlists::remind_closing(&pool).await.unwrap(), 0);
    assert_eq!(status_of(&pool, far).await, "active");
    common::assert_all_reconciled(&pool).await;
}

#[test]
fn closing_mail_renders_taipei_time() {
    let (subject, body) = wishly_api::notify::render(
        "wishlist.closing",
        "小愛的清單",
        &json!({ "closes_at": "2030-01-01T16:30:00Z" }),
    );
    assert!(
        subject.contains("小愛的清單") && subject.contains("即將結束"),
        "{subject}"
    );
    assert!(
        body.contains("2030-01-02 00:30"),
        "UTC 16:30 應顯示為台北 00:30：{body}"
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn expired_list_rejects_claims_before_sweep(pool: PgPool) {
    let _cdn = common::cdn::init();
    let common::TestUser { id: o, token: _ } = common::user(&pool, "u").await;
    let ok = deadline_list(&pool, o, "active", "now() + interval '1 hour'").await;
    let late = deadline_list(&pool, o, "active", "now() - interval '1 minute'").await;
    let mut ids = vec![];
    for w in [ok, late] {
        ids.push(sqlx::query_scalar::<_, Uuid>("INSERT INTO wishlist_items (wishlist_id, title, qty_needed) VALUES ($1, '奶瓶', 2) RETURNING id").bind(w).fetch_one(&pool).await.unwrap());
    }
    let donor = common::user(&pool, "小明").await;
    let claim = |item: Uuid| {
        let pool = pool.clone();
        let token = donor.token.clone();
        async move {
            common::call(
                &pool,
                "POST",
                &format!("/items/{item}/claims"),
                &[common::ck(&token), common::idem(Uuid::new_v4())].concat(),
                Some(json!({ "qty": 1 })),
            )
            .await
        }
    };
    assert_eq!(claim(ids[0]).await.0, StatusCode::CREATED);
    let (s, _, b) = claim(ids[1]).await;
    assert_eq!(
        (s.as_u16(), b["code"].as_str()),
        (409, Some("WISHLIST_CLOSED")),
        "{b}"
    );
    let n: i32 = sqlx::query_scalar("SELECT qty_claimed FROM wishlist_items WHERE id = $1")
        .bind(ids[1])
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 0, "被擋的捐助不留痕");
    common::assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn expired_list_rejects_pledges_before_sweep(pool: PgPool) {
    let _cdn = common::cdn::init();
    let common::TestUser { id: o, token: _ } = common::user(&pool, "u").await;
    let common::TestUser {
        id: donor,
        token: tok,
    } = common::user(&pool, "u").await;
    let wid: Uuid = deadline_list(&pool, o, "active", "now() - interval '1 minute'").await;
    sqlx::query("INSERT INTO shipping_addresses (wishlist_id, recipient_name_enc, phone_enc, address_enc) VALUES ($1, $2, $2, $2)")
        .bind(wid).bind(wishly_api::sealed::seal(b"x")).execute(&pool).await.unwrap();
    let item: Uuid = sqlx::query_scalar(
        "INSERT INTO wishlist_items (wishlist_id, title, qty_needed, funding_mode, target_points, funding_status, funding_deadline, fulfillment_type)
         VALUES ($1, '推車', 1, 'crowdfund', 500, 'open', now() + interval '7 days', 'concierge') RETURNING id").bind(wid).fetch_one(&pool).await.unwrap();
    let wal: Uuid = sqlx::query_scalar(
        "INSERT INTO point_wallets (user_id, balance) VALUES ($1, 100) RETURNING id",
    )
    .bind(donor)
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO point_ledger (wallet_id, delta, balance_after, entry_type, ref_type, note) VALUES ($1, 100, 100, 'grant', 'manual', 't')").bind(wal).execute(&pool).await.unwrap();

    let mut hdr = ck(&tok).to_vec();
    hdr.push(("idempotency-key", Uuid::new_v4().to_string()));
    let (s, _, b) = common::call(
        &pool,
        "POST",
        &format!("/items/{item}/contributions"),
        &hdr,
        Some(json!({ "points": 10 })),
    )
    .await;
    assert!(s.is_client_error(), "{s} {b}");
    let (bal, pledged): (i64, i64) = sqlx::query_as("SELECT (SELECT balance FROM point_wallets WHERE id = $1), (SELECT pledged_points FROM wishlist_items WHERE id = $2)")
        .bind(wal).bind(item).fetch_one(&pool).await.unwrap();
    assert_eq!((bal, pledged), (100, 0), "被擋的認捐不扣點");
    // 對照組：期限改到未來，同一筆認捐即成功（確認上面是被 closes_at 擋，而非 fixture 缺漏）
    sqlx::query("UPDATE wishlists SET closes_at = now() + interval '1 hour' WHERE id = $1")
        .bind(wid)
        .execute(&pool)
        .await
        .unwrap();
    hdr.pop();
    hdr.push(("idempotency-key", Uuid::new_v4().to_string()));
    let (s, _, b) = common::call(
        &pool,
        "POST",
        &format!("/items/{item}/contributions"),
        &hdr,
        Some(json!({ "points": 10 })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    common::assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn public_page_exposes_closes_at(pool: PgPool) {
    let _cdn = common::cdn::init();
    let common::TestUser { id: o, token: _ } = common::user(&pool, "u").await;
    let wid = deadline_list(&pool, o, "active", "'2999-01-01T00:00:00Z'::timestamptz").await;
    sqlx::query(
        "INSERT INTO wishlist_items (wishlist_id, title, qty_needed) VALUES ($1, '奶瓶', 1)",
    )
    .bind(wid)
    .execute(&pool)
    .await
    .unwrap();
    let slug: String = sqlx::query_scalar("SELECT slug::text FROM wishlists WHERE id = $1")
        .bind(wid)
        .fetch_one(&pool)
        .await
        .unwrap();
    let (s, _, b) = common::call(
        &pool,
        "GET",
        &format!("/public/wishlists/{}", slug.trim()),
        &[],
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["closes_at"], "2999-01-01T00:00:00Z");
    common::assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn reopen_requires_future_or_cleared_closes_at(pool: PgPool) {
    let _cdn = common::cdn::init();
    let common::TestUser { id: o, token: t } = common::user(&pool, "u").await;
    let wid = deadline_list(&pool, o, "active", "now() - interval '1 minute'").await;
    wishlists::close_due(&pool).await.unwrap();
    let uri = format!("/wishlists/{wid}");
    let (s, _, b) = common::call(
        &pool,
        "PATCH",
        &uri,
        &ck(&t),
        Some(json!({ "status": "active" })),
    )
    .await;
    assert_eq!(
        (s.as_u16(), b["errors"][0]["pointer"].as_str()),
        (422, Some("/closes_at")),
        "{b}"
    );
    assert_eq!(status_of(&pool, wid).await, "closed");
    let (s, _, b) = common::call(&pool, "PATCH", &uri, &ck(&t), Some(json!({ "status": "active", "closes_at": (chrono::Utc::now() + chrono::Duration::days(1)).to_rfc3339() }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(status_of(&pool, wid).await, "active");
    common::assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn close_due_purges_due_public_lists_and_keeps_future_lists_cached(pool: PgPool) {
    let cdn = common::cdn::init();
    let owner = common::user(&pool, "Owner").await;
    let public = common::wishlist(
        &pool,
        owner.id,
        common::ListOpts {
            visibility: Some("public"),
            closes_in: Some("-1 minute"),
            ..Default::default()
        },
    )
    .await;
    let link = common::wishlist(
        &pool,
        owner.id,
        common::ListOpts {
            closes_in: Some("-1 minute"),
            ..Default::default()
        },
    )
    .await;
    let future = common::wishlist(
        &pool,
        owner.id,
        common::ListOpts {
            visibility: Some("public"),
            closes_in: Some("1 hour"),
            ..Default::default()
        },
    )
    .await;
    let no_deadline = common::wishlist(
        &pool,
        owner.id,
        common::ListOpts {
            visibility: Some("public"),
            ..Default::default()
        },
    )
    .await;
    let draft = common::wishlist(
        &pool,
        owner.id,
        common::ListOpts {
            status: Some("draft"),
            visibility: Some("public"),
            closes_in: Some("-1 minute"),
            ..Default::default()
        },
    )
    .await;
    assert_eq!(wishlists::close_due(&pool).await.unwrap(), 2);
    let purge = cdn.wait_for(&public.slug).await;
    assert_eq!(purge.authorization, "Bearer test-purge-token");
    let mut urls: Vec<String> = serde_json::from_value(purge.body["files"].clone()).unwrap();
    urls.sort();
    let mut expected = vec![
        common::cdn::public_url(&public.slug),
        common::cdn::public_url(&link.slug),
    ];
    expected.sort();
    assert_eq!(
        urls, expected,
        "only the two due active lists should be purged"
    );
    for (list, expected_status) in [
        (&public, "closed"),
        (&link, "closed"),
        (&future, "active"),
        (&no_deadline, "active"),
        (&draft, "draft"),
    ] {
        let state: (String, bool) =
            sqlx::query_as("SELECT status::text,closed_at IS NOT NULL FROM wishlists WHERE id=$1")
                .bind(list.id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(state, (expected_status.into(), expected_status == "closed"));
        let (status, _, body) = common::call(
            &pool,
            "GET",
            &format!("/public/wishlists/{}", list.slug),
            &[],
            None,
        )
        .await;
        if expected_status == "draft" {
            assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
        } else {
            assert_eq!(status, StatusCode::OK, "{body}");
            assert_eq!(body["status"], expected_status);
        }
        if expected_status != "closed" {
            assert!(
                cdn.requests_for(&list.slug).is_empty(),
                "unexpired/draft list was purged"
            );
        }
    }
    assert_eq!(wishlists::close_due(&pool).await.unwrap(), 0);
    assert_eq!(cdn.requests_for(&public.slug).len(), 1);
    assert_eq!(cdn.requests_for(&link.slug).len(), 1);
    common::assert_all_reconciled(&pool).await;
}
