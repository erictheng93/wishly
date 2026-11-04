//! 檢舉保留政策：結案後依規則清除 snapshot 與 detail（列本身保留）；帳號刪除立即清已結案的。
mod common;
use axum::http::StatusCode;
use common::{call, ck, user, wishlist, ListOpts};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

/// 建一筆帶快照與描述的使用者檢舉；handled_days_ago = None 表示待處理
async fn report(
    pool: &PgPool,
    target_user: Option<Uuid>,
    wishlist_id: Option<Uuid>,
    reason: &str,
    status: &str,
    handled_days_ago: Option<i32>,
) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO content_reports (reported_user_id, wishlist_id, reason, detail, snapshot, status, handled_at)
         VALUES ($1, $2, $3::report_reason, 'TEST detail', '{\"title\":\"x\"}'::jsonb, $4::report_status,
                 CASE WHEN $5::int IS NULL THEN NULL ELSE now() - make_interval(days => $5) END) RETURNING id",
    )
    .bind(target_user).bind(wishlist_id).bind(reason).bind(status).bind(handled_days_ago)
    .fetch_one(pool).await.unwrap()
}

async fn kept(pool: &PgPool, id: Uuid) -> (bool, bool, String) {
    sqlx::query_as("SELECT snapshot IS NOT NULL, detail IS NOT NULL, status::text FROM content_reports WHERE id = $1")
        .bind(id).fetch_one(pool).await.unwrap()
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn retention_rules(pool: PgPool) {
    let t = user(&pool, "被檢舉").await.id;
    let l = wishlist(&pool, user(&pool, "擁有者").await.id, ListOpts::default())
        .await
        .id;
    let archived = wishlist(
        &pool,
        user(&pool, "封存者").await.id,
        ListOpts {
            status: Some("archived"),
            ..Default::default()
        },
    )
    .await
    .id;

    let open_old = report(&pool, Some(t), None, "scam", "open", None).await;
    sqlx::query(
        "UPDATE content_reports SET created_at = now() - interval '900 days' WHERE id = $1",
    )
    .bind(open_old)
    .execute(&pool)
    .await
    .unwrap();
    let dismissed_29 = report(&pool, Some(t), None, "scam", "dismissed", Some(29)).await;
    let dismissed_31 = report(&pool, Some(t), None, "scam", "dismissed", Some(31)).await;
    let actioned_364 = report(&pool, None, Some(l), "scam", "actioned", Some(364)).await;
    let actioned_366 = report(&pool, None, Some(l), "scam", "actioned", Some(366)).await;
    let pi_6 = report(&pool, Some(t), None, "personal_info", "actioned", Some(6)).await;
    let pi_8 = report(&pool, Some(t), None, "personal_info", "dismissed", Some(8)).await;
    let archived_8 = report(&pool, None, Some(archived), "scam", "actioned", Some(8)).await;
    let archived_6 = report(&pool, None, Some(archived), "scam", "actioned", Some(6)).await;

    wishly_api::ratelimit::cleanup(&pool).await.unwrap();

    // 待處理不論多舊都留
    assert_eq!(kept(&pool, open_old).await, (true, true, "open".into()));
    assert_eq!(kept(&pool, dismissed_29).await.0, true);
    assert_eq!(
        kept(&pool, dismissed_31).await,
        (false, false, "dismissed".into())
    );
    assert_eq!(kept(&pool, actioned_364).await.0, true);
    assert_eq!(
        kept(&pool, actioned_366).await,
        (false, false, "actioned".into())
    );
    assert_eq!(kept(&pool, pi_6).await.0, true);
    assert_eq!(kept(&pool, pi_8).await.0, false);
    // 對象已封存：比照 7 天
    assert_eq!(kept(&pool, archived_6).await.0, true);
    assert_eq!(kept(&pool, archived_8).await.0, false);
    // 列本身保留（統計與處置紀錄靠它）
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM content_reports")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 9); // 9 筆全在，只清欄位不刪列
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn target_account_deleted_uses_seven_days(pool: PgPool) {
    let t = user(&pool, "已刪").await.id;
    let r6 = report(&pool, Some(t), None, "scam", "actioned", Some(6)).await;
    let r8 = report(&pool, Some(t), None, "scam", "actioned", Some(8)).await;
    sqlx::query("UPDATE users SET deleted_at = now() WHERE id = $1")
        .bind(t)
        .execute(&pool)
        .await
        .unwrap();
    wishly_api::ratelimit::cleanup(&pool).await.unwrap();
    assert!(kept(&pool, r6).await.0);
    assert!(!kept(&pool, r8).await.0);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn account_deletion_clears_closed_reports_now(pool: PgPool) {
    let _cdn = common::cdn::init();
    let me = user(&pool, "要刪帳號").await;
    let other = user(&pool, "旁人").await.id;
    let mine = wishlist(&pool, me.id, ListOpts::default()).await.id;
    let closed_user = report(&pool, Some(me.id), None, "scam", "dismissed", Some(0)).await;
    let closed_list = report(&pool, None, Some(mine), "scam", "actioned", Some(0)).await;
    let open_user = report(&pool, Some(me.id), None, "scam", "open", None).await;
    let unrelated = report(&pool, Some(other), None, "scam", "dismissed", Some(0)).await;

    let (s, _, _) = call(
        &pool,
        "DELETE",
        "/me",
        &ck(&me.token),
        Some(json!({"confirm": "DELETE"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    assert_eq!(
        kept(&pool, closed_user).await,
        (false, false, "dismissed".into())
    );
    assert_eq!(
        kept(&pool, closed_list).await,
        (false, false, "actioned".into())
    );
    // 待處理的留給審查（刪帳號不能讓證據消失）
    assert_eq!(kept(&pool, open_user).await, (true, true, "open".into()));
    assert_eq!(kept(&pool, unrelated).await.0, true);
}
