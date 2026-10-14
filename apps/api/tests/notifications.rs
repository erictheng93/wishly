mod common;
use serde_json::json;
use sqlx::PgPool;

/// 回傳 (user_id, session token)
use common::*;

#[sqlx::test(migrations = "../../db/migrations")]
async fn enqueue_claim_first_immediate_then_digest(pool: PgPool) {
    let TestUser {
        id: owner,
        token: _,
    } = user(&pool, "o").await;
    let (wid, _, _) = claimed_wishlist(&pool, owner, true, 30).await;
    let mut tx = pool.begin().await.unwrap();
    for _ in 0..3 {
        wishsync_api::notify::enqueue_claim(&mut *tx, wid)
            .await
            .unwrap();
    }
    let rows: Vec<(String, i32, bool)> = sqlx::query_as(
        "SELECT kind, (payload->>'count')::int, CASE WHEN kind='claim.created' THEN scheduled_at=now() ELSE scheduled_at=((date_trunc('day', now() AT TIME ZONE 'Asia/Taipei') + interval '9 hours' + CASE WHEN (now() AT TIME ZONE 'Asia/Taipei')::time >= time '09:00' THEN interval '1 day' ELSE interval '0' END) AT TIME ZONE 'Asia/Taipei') END FROM notifications WHERE user_id = $1 ORDER BY kind").bind(owner).fetch_all(&mut *tx).await.unwrap();
    assert_eq!(
        rows,
        vec![
            ("claim.created".to_string(), 1, true),
            ("claim.digest".to_string(), 2, true)
        ]
    );
    tx.commit().await.unwrap();
    // 關閉通知則不寫
    sqlx::query("UPDATE users SET notification_prefs = '{\"email_claims\": false}' WHERE id = $1")
        .bind(owner)
        .execute(&pool)
        .await
        .unwrap();
    wishsync_api::notify::enqueue_claim(&pool, wid)
        .await
        .unwrap();
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM notifications")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 2);
    let (_, body) = wishsync_api::notify::render("claim.digest", "清單", &json!({"count": 2}));
    assert!(body.contains("2 件新認領"));
    assert_all_reconciled(&pool).await;
}
