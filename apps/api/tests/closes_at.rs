//! 清單自動結束時間 closes_at：建立 / 驗證、到期掃描、到期前提醒、認領與認捐即時擋、公開頁。
mod common;
use axum::http::StatusCode;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;
use wishsync_api::wishlists;

async fn call(
    pool: &PgPool,
    method: &str,
    uri: &str,
    hdr: &[(&str, String)],
    body: Option<Value>,
) -> (StatusCode, Value) {
    let (s, _, b) = common::api(pool, method, uri, hdr, body).await;
    (s, b)
}
use common::ck;

async fn user(pool: &PgPool) -> (Uuid, String) {
    common::user(pool, "u").await
}

/// 清單；closes_sql 是 SQL 運算式（如 "now() + interval '1 hour'" 或 "NULL"）
async fn list(pool: &PgPool, owner: Uuid, status: &str, closes_sql: &str) -> Uuid {
    let slug: String = Uuid::new_v4().simple().to_string()[..10].to_string();
    sqlx::query_scalar(&format!(
        "INSERT INTO wishlists (owner_id, type, visibility, status, slug, title, closes_at)
         VALUES ($1, 'registry', 'link', $2::wishlist_status, $3, '清單', {closes_sql}) RETURNING id"))
        .bind(owner).bind(status).bind(slug).fetch_one(pool).await.unwrap()
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
    let (_, t) = user(&pool).await;
    let (s, b) = call(
        &pool,
        "POST",
        "/wishlists",
        &ck(&t),
        Some(json!({ "type": "personal", "title": "不設期限" })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    assert!(b["closes_at"].is_null(), "預設不設期限");

    let (s, b) = call(
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
        let (s, b) = call(
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
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn patch_sets_clears_and_keeps_closes_at(pool: PgPool) {
    let (o, t) = user(&pool).await;
    let wid = list(&pool, o, "active", "NULL").await;
    let uri = format!("/wishlists/{wid}");
    let (_, b) = call(
        &pool,
        "PATCH",
        &uri,
        &ck(&t),
        Some(json!({ "closes_at": "2999-01-01T00:00:00Z" })),
    )
    .await;
    assert_eq!(b["closes_at"], "2999-01-01T00:00:00Z");
    // 不帶欄位 = 不動
    let (_, b) = call(
        &pool,
        "PATCH",
        &uri,
        &ck(&t),
        Some(json!({ "title": "改名" })),
    )
    .await;
    assert_eq!(b["closes_at"], "2999-01-01T00:00:00Z");
    // null = 清除
    let (_, b) = call(
        &pool,
        "PATCH",
        &uri,
        &ck(&t),
        Some(json!({ "closes_at": null })),
    )
    .await;
    assert!(b["closes_at"].is_null());
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn close_due_only_touches_active_and_expired(pool: PgPool) {
    let (o, _) = user(&pool).await;
    let due = list(&pool, o, "active", "now() - interval '1 minute'").await;
    let future = list(&pool, o, "active", "now() + interval '1 hour'").await;
    let none = list(&pool, o, "active", "NULL").await;
    let draft = list(&pool, o, "draft", "now() - interval '1 minute'").await;
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
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn reminder_sent_once_and_rearmed_on_change(pool: PgPool) {
    let (o, t) = user(&pool).await;
    let soon = list(&pool, o, "active", "now() + interval '2 hours'").await;
    let far = list(&pool, o, "active", "now() + interval '3 days'").await;
    let _none = list(&pool, o, "active", "NULL").await;
    let _closed = list(&pool, o, "closed", "now() + interval '2 hours'").await;
    let _past = list(&pool, o, "active", "now() - interval '1 hour'").await;

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
    let (s, _) = call(
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
    let (s, _) = call(
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
}

#[test]
fn closing_mail_renders_taipei_time() {
    let (subject, body) = wishsync_api::notify::render(
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
    let (o, _) = user(&pool).await;
    let ok = list(&pool, o, "active", "now() + interval '1 hour'").await;
    let late = list(&pool, o, "active", "now() - interval '1 minute'").await;
    let mut ids = vec![];
    for w in [ok, late] {
        ids.push(sqlx::query_scalar::<_, Uuid>("INSERT INTO wishlist_items (wishlist_id, title, qty_needed) VALUES ($1, '奶瓶', 2) RETURNING id").bind(w).fetch_one(&pool).await.unwrap());
    }
    let claim = |item: Uuid| {
        let pool = pool.clone();
        async move {
            call(
                &pool,
                "POST",
                &format!("/items/{item}/claims"),
                &[("idempotency-key", Uuid::new_v4().to_string())],
                Some(json!({ "qty": 1, "display_name": "小明" })),
            )
            .await
        }
    };
    assert_eq!(claim(ids[0]).await.0, StatusCode::CREATED);
    let (s, b) = claim(ids[1]).await;
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
    assert_eq!(n, 0, "被擋的認領不留痕");
    common::assert_claims_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn expired_list_rejects_pledges_before_sweep(pool: PgPool) {
    let (o, _) = user(&pool).await;
    let (donor, tok) = user(&pool).await;
    let wid: Uuid = list(&pool, o, "active", "now() - interval '1 minute'").await;
    sqlx::query("INSERT INTO shipping_addresses (wishlist_id, recipient_name_enc, phone_enc, address_enc) VALUES ($1, $2, $2, $2)")
        .bind(wid).bind(wishsync_api::sealed::seal(b"x")).execute(&pool).await.unwrap();
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
    let (s, b) = call(
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
    let (s, b) = call(
        &pool,
        "POST",
        &format!("/items/{item}/contributions"),
        &hdr,
        Some(json!({ "points": 10 })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn public_page_exposes_closes_at(pool: PgPool) {
    let (o, _) = user(&pool).await;
    let wid = list(&pool, o, "active", "'2999-01-01T00:00:00Z'::timestamptz").await;
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
    let (s, b) = call(
        &pool,
        "GET",
        &format!("/public/wishlists/{}", slug.trim()),
        &[],
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["closes_at"], "2999-01-01T00:00:00Z");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn reopen_requires_future_or_cleared_closes_at(pool: PgPool) {
    let (o, t) = user(&pool).await;
    let wid = list(&pool, o, "active", "now() - interval '1 minute'").await;
    wishlists::close_due(&pool).await.unwrap();
    let uri = format!("/wishlists/{wid}");
    let (s, b) = call(
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
    let (s, b) = call(&pool, "PATCH", &uri, &ck(&t), Some(json!({ "status": "active", "closes_at": (chrono::Utc::now() + chrono::Duration::days(1)).to_rfc3339() }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(status_of(&pool, wid).await, "active");
}
