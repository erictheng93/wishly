-- 通知中心：站內通知欄位、活動摘要去重欄位、通知偏好鍵整併。
ALTER TABLE notifications
  ADD COLUMN inbox_at    timestamptz,
  ADD COLUMN inbox_title text,
  ADD COLUMN inbox_link  text,
  ADD COLUMN read_at     timestamptz;
CREATE INDEX notifications_inbox_idx ON notifications (user_id, inbox_at DESC) WHERE inbox_at IS NOT NULL;

ALTER TABLE wishlists ADD COLUMN event_summary_sent_at timestamptz;

-- 偏好鍵：email_claims 作廢（預設即開）；email_birthdays 改名 email_social；新增 email_gifts（未設視為開）
ALTER TABLE users ALTER COLUMN notification_prefs SET DEFAULT '{}'::jsonb;
UPDATE users
   SET notification_prefs = (notification_prefs - 'email_claims' - 'email_birthdays')
         || CASE WHEN notification_prefs ? 'email_birthdays'
                 THEN jsonb_build_object('email_social', notification_prefs->'email_birthdays')
                 ELSE '{}'::jsonb END
 WHERE notification_prefs ? 'email_claims' OR notification_prefs ? 'email_birthdays';

-- 外鍵檢查會對 users 取 FOR KEY SHARE：持有 topups / 錢包 / 品項鎖的交易寫通知時，
-- 會與帳號刪除的 users（FOR UPDATE）→ topups → 錢包 鎖序相反而死鎖。users 只軟刪除，worker 也會處理收件人不存在。
ALTER TABLE notifications DROP CONSTRAINT notifications_user_id_fkey;
