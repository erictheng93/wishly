//! Q12：認領購買期限（預設 7 天 / 活動日前 3 天、至少 48 小時）、提醒信、延長一次、逾期釋出與通知。
mod common;
use axum::http::StatusCode;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use common::*;

/// 個人清單＋一個品項；ttl 以 SQL 設定（common::wishlist 直接 INSERT，不經 API 預設）
async fn setup(
    pool: &PgPool,
    ttl: Option<i32>,
    event_offset_days: Option<i32>,
) -> (TestUser, Uuid, Uuid) {
    let o = user(pool, "擁有者").await;
    let wl = wishlist(
        pool,
        o.id,
        ListOpts {
            event_offset_days,
            ..Default::default()
        },
    )
    .await
    .id;
    sqlx::query("UPDATE wishlists SET claim_ttl_hours = $2 WHERE id = $1")
        .bind(wl)
        .bind(ttl)
        .execute(pool)
        .await
        .unwrap();
    let it = item(pool, wl, "奶瓶", 10).await;
    (o, wl, it)
}

/// expires_at - created_at（秒）；NULL = 不限
async fn window_secs(pool: &PgPool, c: Uuid) -> Option<i64> {
    sqlx::query_scalar(
        "SELECT extract(epoch FROM expires_at - created_at)::bigint FROM claims WHERE id = $1",
    )
    .bind(c)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// 每封提醒距離截止的秒數（依排程先後）
async fn reminders(pool: &PgPool, c: Uuid, status: &str) -> Vec<i64> {
    sqlx::query_scalar(
        "SELECT extract(epoch FROM (payload->>'expires_at')::timestamptz - scheduled_at)::bigint FROM notifications
          WHERE kind = 'claim.purchase_reminder' AND payload->>'claim_id' = $1 AND status::text = $2 ORDER BY scheduled_at")
        .bind(c.to_string()).bind(status).fetch_all(pool).await.unwrap()
}

async fn patch(pool: &PgPool, c: Uuid, tok: &str, body: Value) -> Resp {
    call(pool, "PATCH", &format!("/claims/{c}"), &ck(tok), Some(body)).await
}

const H: i64 = 3600;
const D: i64 = 24 * H;

#[sqlx::test(migrations = "../../db/migrations")]
async fn window_defaults_event_rule_and_unlimited(pool: PgPool) {
    // 預設 7 天，提醒在截止前 3 天與 24 小時
    let (_, _, it) = setup(&pool, Some(168), None).await;
    let (_, c) = user_claim(&pool, it, 1).await;
    assert_eq!(window_secs(&pool, c).await, Some(7 * D));
    assert_eq!(reminders(&pool, c, "pending").await, vec![3 * D, D]);

    // 14 天設定
    let (_, _, it) = setup(&pool, Some(336), None).await;
    let (_, c) = user_claim(&pool, it, 1).await;
    assert_eq!(window_secs(&pool, c).await, Some(14 * D));

    // 活動日 7 天後：改用活動日台北 00:00 前 3 天（比 7 天早、且必定超過 48 小時）
    let (_, _, it) = setup(&pool, Some(168), Some(7)).await;
    let (_, c) = user_claim(&pool, it, 1).await;
    let exact: bool = sqlx::query_scalar(
        "SELECT c.expires_at = (w.event_date::timestamp AT TIME ZONE 'Asia/Taipei') - interval '3 days'
           FROM claims c JOIN wishlist_items i ON i.id = c.item_id JOIN wishlists w ON w.id = i.wishlist_id WHERE c.id = $1")
        .bind(c).fetch_one(&pool).await.unwrap();
    assert!(exact);
    assert!(window_secs(&pool, c).await.unwrap() < 7 * D);

    // 活動日明天：前 3 天已過 → 至少 48 小時；只剩 24 小時那封提醒
    let (_, _, it) = setup(&pool, Some(168), Some(1)).await;
    let (_, c) = user_claim(&pool, it, 1).await;
    assert_eq!(window_secs(&pool, c).await, Some(48 * H));
    assert_eq!(reminders(&pool, c, "pending").await, vec![D]);

    // 不限：沒有期限也沒有提醒
    let (_, _, it) = setup(&pool, None, Some(1)).await;
    let (_, c) = user_claim(&pool, it, 1).await;
    assert_eq!(window_secs(&pool, c).await, None);
    assert!(reminders(&pool, c, "pending").await.is_empty());
    assert_claims_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn new_list_defaults_to_seven_days_and_null_means_unlimited(pool: PgPool) {
    let o = user(&pool, "o").await;
    let (s, _, b) = call(
        &pool,
        "POST",
        "/wishlists",
        &ck(&o.token),
        Some(json!({"type":"personal","title":"生日"})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    assert_eq!(b["claim_ttl_hours"], 168);
    let (s, _, b) = call(
        &pool,
        "POST",
        "/wishlists",
        &ck(&o.token),
        Some(json!({"type":"personal","title":"生日","claim_ttl_hours":null})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    assert_eq!(b["claim_ttl_hours"], Value::Null);
    let (s, _, b) = call(
        &pool,
        "POST",
        "/wishlists",
        &ck(&o.token),
        Some(json!({"type":"personal","title":"生日","claim_ttl_hours":720})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    assert_eq!(b["claim_ttl_hours"], 720);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn extend_once_by_seven_days(pool: PgPool) {
    let (o, wl, it) = setup(&pool, Some(168), None).await;
    let (t, c) = user_claim(&pool, it, 1).await;
    let stranger = user(&pool, "路人").await;
    assert_eq!(
        patch(&pool, c, &stranger.token, json!({"extend":true}))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        patch(&pool, c, &o.token, json!({"extend":true})).await.0,
        StatusCode::FORBIDDEN
    );

    let (s, _, b) = patch(&pool, c, &t, json!({"extend":true})).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert!(b["claim"]["extended_at"].is_string());
    assert_eq!(window_secs(&pool, c).await, Some(14 * D));
    // 新期限重排提醒（舊的兩封留著，寄信時會被取消）
    let new_ones: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM notifications n JOIN claims c ON c.id = (n.payload->>'claim_id')::uuid
          WHERE n.kind = 'claim.purchase_reminder' AND c.id = $1 AND (n.payload->>'expires_at')::timestamptz = c.expires_at")
        .bind(c).fetch_one(&pool).await.unwrap();
    assert_eq!(new_ones, 2);

    let (s, _, b) = patch(&pool, c, &t, json!({"extend":true})).await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert_eq!(b["code"], "EXTENSION_USED");
    assert_eq!(window_secs(&pool, c).await, Some(14 * D));

    // 已購買 → 不允許
    let (t2, c2) = user_claim(&pool, it, 1).await;
    assert_eq!(
        patch(&pool, c2, &t2, json!({"status":"purchased"})).await.0,
        StatusCode::OK
    );
    let (s, _, b) = patch(&pool, c2, &t2, json!({"extend":true})).await;
    assert_eq!(
        (s, b["code"].as_str()),
        (StatusCode::CONFLICT, Some("INVALID_STATE_TRANSITION"))
    );

    // 清單結束：只能取消（Q2）
    let (t3, c3) = user_claim(&pool, it, 1).await;
    sqlx::query("UPDATE wishlists SET status = 'closed', closed_at = now() WHERE id = $1")
        .bind(wl)
        .execute(&pool)
        .await
        .unwrap();
    let (s, _, b) = patch(&pool, c3, &t3, json!({"extend":true})).await;
    assert_eq!(
        (s, b["code"].as_str()),
        (StatusCode::CONFLICT, Some("WISHLIST_CLOSED"))
    );
    assert_eq!(window_secs(&pool, c3).await, Some(7 * D));

    // 不限期限的清單 → 沒有可延長的期限
    let (_, _, it4) = setup(&pool, None, None).await;
    let (t4, c4) = user_claim(&pool, it4, 1).await;
    let (s, _, b) = patch(&pool, c4, &t4, json!({"extend":true})).await;
    assert_eq!(
        (s, b["code"].as_str()),
        (StatusCode::CONFLICT, Some("INVALID_STATE_TRANSITION"))
    );
    assert_claims_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn stale_reminders_are_cancelled_at_send_time(pool: PgPool) {
    let (_, _, it) = setup(&pool, Some(168), None).await;
    let (tb, bought) = user_claim(&pool, it, 1).await;
    let (te, extended) = user_claim(&pool, it, 1).await;
    assert_eq!(
        patch(&pool, bought, &tb, json!({"status":"purchased"}))
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        patch(&pool, extended, &te, json!({"extend":true})).await.0,
        StatusCode::OK
    );
    // 全部提早到期，交給 worker 判斷
    sqlx::query(
        "UPDATE notifications SET scheduled_at = now() WHERE kind = 'claim.purchase_reminder'",
    )
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(wishly_api::notify::tick(&pool).await.unwrap(), 6);
    assert_eq!(reminders(&pool, bought, "cancelled").await.len(), 2);
    assert_eq!(reminders(&pool, extended, "cancelled").await.len(), 2); // 延長前的舊期限
    assert_eq!(reminders(&pool, extended, "sent").await.len(), 2); // 新期限
}

#[test]
fn reminder_and_expiry_copy() {
    use wishly_api::notify::{render_lang, Lang};
    let p = json!({ "item_title": "奶瓶", "expires_at": "2026-10-29T04:00:00Z" });
    let (s, b) = render_lang(Lang::Zh, "claim.purchase_reminder", "寶寶清單", &p);
    assert!(s.contains("奶瓶"), "{s}");
    assert!(
        b.contains("還沒買嗎？")
            && b.contains("2026-10-29 12:00")
            && b.contains("延長 7 天")
            && b.contains("/me/claims"),
        "{b}"
    );
    for l in [Lang::En, Lang::Vi] {
        let (_, b) = render_lang(l, "claim.purchase_reminder", "L", &p);
        assert!(
            b.contains("2026-10-29 12:00") && b.contains("/me/claims"),
            "{b}"
        );
    }
    let (_, b) = render_lang(Lang::Zh, "claim.expired", "寶寶清單", &p);
    assert!(
        b.contains("你在《寶寶清單》認領的「奶瓶」已超過購買期限，已釋出給其他朋友"),
        "{b}"
    );
    let (s, _) = render_lang(Lang::Zh, "item.reopened", "寶寶清單", &p);
    assert!(s.contains("《寶寶清單》有品項重新開放認領"), "{s}");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn expiry_releases_on_active_and_closed_lists_and_notifies(pool: PgPool) {
    let (o, active, a) = setup(&pool, Some(168), None).await;
    let (_, closed, b) = setup(&pool, Some(168), None).await;
    let (_, a1) = user_claim(&pool, a, 2).await;
    let (_, a2) = user_claim(&pool, a, 1).await;
    let (_, b1) = user_claim(&pool, b, 3).await;
    let (tp, bought) = user_claim(&pool, a, 1).await;
    assert_eq!(
        patch(&pool, bought, &tp, json!({"status":"purchased"}))
            .await
            .0,
        StatusCode::OK
    );
    let exp: Option<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar("SELECT expires_at FROM claims WHERE id = $1")
            .bind(bought)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(exp.is_none(), "已購買不會逾期");
    sqlx::query("UPDATE wishlists SET status = 'closed', closed_at = now() WHERE id = $1")
        .bind(closed)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE claims SET expires_at = now() - interval '1 minute' WHERE status = 'reserved'",
    )
    .execute(&pool)
    .await
    .unwrap();

    assert_eq!(wishly_api::claims::expire_due(&pool).await.unwrap(), 3);
    let st: Vec<(Uuid, String)> =
        sqlx::query_as("SELECT id, status::text FROM claims WHERE id = ANY($1) ORDER BY id")
            .bind(vec![a1, a2, b1, bought])
            .fetch_all(&pool)
            .await
            .unwrap();
    for (id, s) in st {
        assert_eq!(s, if id == bought { "purchased" } else { "expired" });
    }
    let q: Vec<i32> = sqlx::query_scalar(
        "SELECT qty_claimed FROM wishlist_items WHERE id = ANY($1) ORDER BY id = $2 DESC",
    )
    .bind(vec![a, b])
    .bind(a)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(q, vec![1, 0]);
    let ws: String = sqlx::query_scalar("SELECT status::text FROM wishlists WHERE id = $1")
        .bind(closed)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(ws, "closed", "逾期釋出不會重開清單");
    let _ = active;

    // 捐助者各一封；擁有者每個品項一封，不含捐助者資訊
    let donor: i64 =
        sqlx::query_scalar("SELECT count(*) FROM notifications WHERE kind = 'claim.expired'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(donor, 3);
    let owner: Vec<(Uuid, Value)> = sqlx::query_as("SELECT user_id, payload FROM notifications WHERE kind = 'item.reopened' ORDER BY payload->>'item_id' = $1 DESC")
        .bind(a.to_string()).fetch_all(&pool).await.unwrap();
    assert_eq!(owner.len(), 2);
    assert_eq!(owner[0].0, o.id);
    for (_, p) in &owner {
        let mut keys: Vec<&str> = p.as_object().unwrap().keys().map(String::as_str).collect();
        keys.sort();
        assert_eq!(keys, vec!["item_id", "wishlist_id"]);
        assert!(!p.to_string().contains("小明"));
    }
    // 再跑不重複通知
    assert_eq!(wishly_api::claims::expire_due(&pool).await.unwrap(), 0);
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM notifications WHERE kind IN ('claim.expired', 'item.reopened')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(n, 5);
    assert_claims_reconciled(&pool).await;
}

/// 期限是否落在 now()+7 天（容許測試執行時間）
async fn graced(pool: &PgPool, c: Uuid) -> bool {
    sqlx::query_scalar("SELECT expires_at BETWEEN now() + interval '7 days' - interval '1 minute' AND now() + interval '7 days' FROM claims WHERE id = $1")
        .bind(c).fetch_one(pool).await.unwrap()
}

async fn expires(pool: &PgPool, c: Uuid) -> Option<chrono::DateTime<chrono::Utc>> {
    sqlx::query_scalar("SELECT expires_at FROM claims WHERE id = $1")
        .bind(c)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn closing_a_list_grants_seven_day_grace(pool: PgPool) {
    for how in ["patch_closed", "patch_archived", "delete", "auto"] {
        let (o, wl, it) = setup(&pool, Some(168), None).await;
        let (_, near) = user_claim(&pool, it, 1).await;
        let (_, overdue) = user_claim(&pool, it, 1).await;
        let (_, far) = user_claim(&pool, it, 1).await;
        let (_, unlimited) = user_claim(&pool, it, 1).await;
        let (tb, bought) = user_claim(&pool, it, 1).await;
        assert_eq!(
            patch(&pool, bought, &tb, json!({"status":"purchased"}))
                .await
                .0,
            StatusCode::OK
        );
        for (c, e) in [
            (near, "now() + interval '2 days'"),
            (overdue, "now() - interval '1 minute'"),
            (far, "now() + interval '10 days'"),
            (unlimited, "NULL"),
        ] {
            sqlx::query(sqlx::AssertSqlSafe(format!(
                "UPDATE claims SET expires_at = {e} WHERE id = $1"
            )))
            .bind(c)
            .execute(&pool)
            .await
            .unwrap();
        }
        let far_before = expires(&pool, far).await;
        match how {
            "patch_closed" | "patch_archived" => {
                let s = if how == "patch_closed" {
                    "closed"
                } else {
                    "archived"
                };
                let (st, _, b) = call(
                    &pool,
                    "PATCH",
                    &format!("/wishlists/{wl}"),
                    &ck(&o.token),
                    Some(json!({ "status": s })),
                )
                .await;
                assert_eq!(st, StatusCode::OK, "{how}: {b}");
            }
            "delete" => {
                let (st, _, b) = call(
                    &pool,
                    "DELETE",
                    &format!("/wishlists/{wl}"),
                    &ck(&o.token),
                    None,
                )
                .await;
                assert_eq!(st, StatusCode::NO_CONTENT, "{how}: {b}");
            }
            _ => {
                sqlx::query(
                    "UPDATE wishlists SET closes_at = now() - interval '1 second' WHERE id = $1",
                )
                .bind(wl)
                .execute(&pool)
                .await
                .unwrap();
                assert_eq!(wishly_api::wishlists::close_due(&pool).await.unwrap(), 1);
            }
        }
        assert!(graced(&pool, near).await, "{how}");
        assert!(graced(&pool, overdue).await, "{how}");
        assert_eq!(expires(&pool, far).await, far_before, "{how}: 不縮短");
        assert_eq!(expires(&pool, unlimited).await, None, "{how}");
        assert_eq!(expires(&pool, bought).await, None, "{how}");
        let notified: Vec<String> = sqlx::query_scalar(
            "SELECT payload->>'claim_id' FROM notifications WHERE kind = 'claim.list_closed' AND payload->>'wishlist_id' = $1 ORDER BY 1")
            .bind(wl.to_string()).fetch_all(&pool).await.unwrap();
        let mut want = vec![near.to_string(), overdue.to_string()];
        want.sort();
        assert_eq!(notified, want, "{how}");
        for c in [near, overdue] {
            let fresh: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM notifications n JOIN claims c ON c.id = (n.payload->>'claim_id')::uuid
                  WHERE n.kind = 'claim.purchase_reminder' AND c.id = $1 AND n.status = 'pending' AND (n.payload->>'expires_at')::timestamptz = c.expires_at")
                .bind(c).fetch_one(&pool).await.unwrap();
            assert_eq!(fresh, 2, "{how}");
        }
        // 重開不縮短期限
        if how == "patch_closed" {
            let (st, _, b) = call(
                &pool,
                "PATCH",
                &format!("/wishlists/{wl}"),
                &ck(&o.token),
                Some(json!({ "status": "active" })),
            )
            .await;
            assert_eq!(st, StatusCode::OK, "{b}");
            assert!(graced(&pool, near).await);
        }
    }
    assert_claims_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn after_grace_unmarked_claims_expire_but_marked_ones_stay(pool: PgPool) {
    let (o, wl, it) = setup(&pool, Some(168), None).await;
    let (_, lazy) = user_claim(&pool, it, 2).await;
    let (tb, bought) = user_claim(&pool, it, 1).await;
    let (st, _, b) = call(
        &pool,
        "PATCH",
        &format!("/wishlists/{wl}"),
        &ck(&o.token),
        Some(json!({ "status": "closed" })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{b}");
    // 清單結束後捐助者仍可標記已購買（CTO 決議），之後永不逾期
    let (st, _, b) = patch(&pool, bought, &tb, json!({"status":"purchased"})).await;
    assert_eq!(st, StatusCode::OK, "{b}");
    assert_eq!(expires(&pool, bought).await, None);
    // 寬限到期
    sqlx::query("UPDATE claims SET expires_at = now() - interval '1 minute' WHERE id = $1")
        .bind(lazy)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(wishly_api::claims::expire_due(&pool).await.unwrap(), 1);
    let s: Vec<String> = sqlx::query_scalar(
        "SELECT status::text FROM claims WHERE id = ANY($1) ORDER BY id = $2 DESC",
    )
    .bind(vec![lazy, bought])
    .bind(lazy)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(s, vec!["expired", "purchased"]);
    let q: i32 = sqlx::query_scalar("SELECT qty_claimed FROM wishlist_items WHERE id = $1")
        .bind(it)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(q, 1);
    let ws: String = sqlx::query_scalar("SELECT status::text FROM wishlists WHERE id = $1")
        .bind(wl)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(ws, "closed");
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM notifications WHERE kind IN ('claim.expired', 'item.reopened')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(n, 2);
    assert_claims_reconciled(&pool).await;
}

#[test]
fn list_closed_copy() {
    use wishly_api::notify::{render_lang, Lang};
    let p = json!({ "item_title": "奶瓶", "expires_at": "2026-10-29T04:00:00Z" });
    let (_, b) = render_lang(Lang::Zh, "claim.list_closed", "寶寶清單", &p);
    assert!(b.contains("清單《寶寶清單》已結束。如果你已經買好或送出了「奶瓶」，請到「我的捐助」標記已購買或已送達；2026-10-29 12:00"), "{b}");
    for l in [Lang::En, Lang::Vi] {
        let (_, b) = render_lang(l, "claim.list_closed", "L", &p);
        assert!(
            b.contains("2026-10-29 12:00") && b.contains("/me/claims"),
            "{b}"
        );
    }
}

/// Q14：清單已結束時的到期提醒不能再叫捐助者「延長」（已結束的清單不能延長），改提示標記已購買 / 已送達
#[test]
fn reminder_on_ended_list_does_not_offer_extension() {
    use wishly_api::notify::{render_lang, Lang};
    let base = json!({ "item": "奶瓶", "expires_at": "2026-11-01T00:00:00Z" });
    let mut ended = base.clone();
    ended["ended"] = json!(true);
    for lang in [Lang::parse("zh-TW"), Lang::parse("en"), Lang::parse("vi")] {
        let (_, open) = render_lang(lang, "claim.purchase_reminder", "清單", &base);
        let (_, closed) = render_lang(lang, "claim.purchase_reminder", "清單", &ended);
        assert!(open.contains('7'), "{open}");
        assert!(!closed.contains('7'), "{closed}");
    }
}
