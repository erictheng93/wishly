-- 組織邀請可寄給尚未註冊的 email：沒有 user / guest 時，收件者放在 payload.to。
ALTER TABLE notifications
  DROP CONSTRAINT notifications_has_recipient,
  ADD CONSTRAINT notifications_has_recipient CHECK (user_id IS NOT NULL OR guest_id IS NOT NULL OR payload ? 'to');
