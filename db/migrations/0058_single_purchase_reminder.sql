-- 購買提醒只留截止前 24 小時一封：取消已排程、尚未寄出的 3 天前提醒。
-- 3 天前那封的 scheduled_at 約為期限前 72 小時（安靜時段最多延後 12 小時），24 小時那封不會早於期限前 24 小時，以 36 小時為界。
UPDATE notifications SET status = 'cancelled', last_error = 'policy: single purchase reminder'
 WHERE kind = 'claim.purchase_reminder' AND status = 'pending'
   AND scheduled_at < (payload->>'expires_at')::timestamptz - interval '36 hours';
