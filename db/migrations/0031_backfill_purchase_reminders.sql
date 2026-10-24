-- 0030 上線前就有期限的 reserved 捐助（relief / 自訂期限清單）補排購買提醒；條件同 notify::enqueue_purchase_reminders。
-- 只補還在未來的，且跳過已排過同一期限的；claims.expires_at 照舊不動
INSERT INTO notifications (user_id, guest_id, channel, kind, payload, scheduled_at)
SELECT c.user_id, c.guest_id, 'email'::notification_channel, 'claim.purchase_reminder',
       jsonb_build_object('wishlist_id', i.wishlist_id, 'item_id', c.item_id, 'claim_id', c.id, 'expires_at', c.expires_at),
       c.expires_at - d.before
  FROM claims c JOIN wishlist_items i ON i.id = c.item_id,
       (VALUES (interval '3 days'), (interval '24 hours')) d(before)
 WHERE c.status = 'reserved' AND c.expires_at - d.before > now()
   AND NOT EXISTS (SELECT 1 FROM notifications n WHERE n.kind = 'claim.purchase_reminder'
                     AND n.payload->>'claim_id' = c.id::text AND n.scheduled_at = c.expires_at - d.before);
