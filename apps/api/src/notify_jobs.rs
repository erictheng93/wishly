//! 排程通知：活動後摘要、營運每日待辦。由 `ratelimit::spawn_cleanup` 的每小時分支呼叫；
//! 唯一性靠條件式 UPDATE ... IS NULL RETURNING / NOT EXISTS，多實例安全，不使用 FOR UPDATE / ON CONFLICT。
use sqlx::PgPool;

/// 活動日（台北）落在今天往前 7 天內、尚未寄過、未鎖定的個人清單，對擁有者寄 `event.summary`（只帶去重人數）。
/// 鎖定清單排除在 UPDATE 之外，才不會提早用掉 event_summary_sent_at。
pub async fn event_summary_tick(pool: &PgPool) -> Result<u64, sqlx::Error> {
    let r = sqlx::query(
        "WITH g AS (
           SELECT w.id, w.owner_id,
                  (SELECT count(DISTINCT r.actor) FROM (
                     SELECT COALESCE(c.user_id, c.guest_id) AS actor FROM claims c JOIN wishlist_items i ON i.id = c.item_id
                      WHERE i.wishlist_id = w.id AND c.status IN ('reserved', 'purchased', 'delivered')
                     UNION ALL
                     SELECT k.user_id FROM contributions k WHERE k.wishlist_id = w.id AND k.status = 'captured') r
                    WHERE r.actor IS NOT NULL AND r.actor <> w.owner_id) AS givers
             FROM wishlists w
            WHERE w.event_summary_sent_at IS NULL AND w.org_id IS NULL AND w.deleted_at IS NULL
              AND w.event_date BETWEEN (now() AT TIME ZONE 'Asia/Taipei')::date - 7 AND (now() AT TIME ZONE 'Asia/Taipei')::date
              AND NOT surprise_locked(w.surprise_mode, w.event_date)),
         u AS (UPDATE wishlists w SET event_summary_sent_at = now() FROM g
                WHERE w.id = g.id AND g.givers > 0 AND w.event_summary_sent_at IS NULL
            RETURNING w.id, w.owner_id, g.givers)
         INSERT INTO notifications (user_id, channel, kind, payload)
         SELECT owner_id, 'email'::notification_channel, 'event.summary', jsonb_build_object('wishlist_id', id, 'givers', givers) FROM u")
        .execute(pool).await?;
    Ok(r.rows_affected())
}

/// 有待辦才寄，給每位未封鎖的 staff；同一人 20 小時內已有 `staff.digest` 就略過。
pub async fn staff_digest_tick(pool: &PgPool) -> Result<u64, sqlx::Error> {
    let r = sqlx::query(
        "WITH c AS (SELECT
            (SELECT count(*) FROM content_reports WHERE status = 'open') AS reports,
            (SELECT count(*) FROM cash_refund_requests WHERE status = 'requested') AS refunds,
            (SELECT count(*) FROM organizations WHERE verification_status = 'pending') AS orgs,
            (SELECT count(*) FROM org_permits WHERE status = 'pending') AS permits,
            (SELECT count(*) FROM purchase_orders WHERE status = 'pending') AS orders,
            (SELECT count(*) FROM notifications WHERE status = 'failed' AND scheduled_at > now() - interval '7 days') AS failed)
         INSERT INTO notifications (user_id, channel, kind, payload)
         SELECT u.id, 'email'::notification_channel, 'staff.digest',
                jsonb_build_object('reports', c.reports, 'refunds', c.refunds, 'orgs', c.orgs,
                                   'permits', c.permits, 'orders', c.orders, 'failed', c.failed)
           FROM users u, c
          WHERE u.is_staff AND u.blocked_at IS NULL AND u.deleted_at IS NULL
            AND c.reports + c.refunds + c.orgs + c.permits + c.orders + c.failed > 0
            AND NOT EXISTS (SELECT 1 FROM notifications n WHERE n.user_id = u.id AND n.kind = 'staff.digest'
                             AND n.created_at > now() - interval '20 hours')")
        .execute(pool).await?;
    Ok(r.rows_affected())
}
