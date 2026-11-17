-- 簡易客服工單（#92）：會員（user_id）或未登入者（contact_email，未經驗證）送出，營運單次回覆即結案。
-- user_id / wishlist_id / topup_id 刻意不加外鍵（理由同 0050：外鍵檢查會鎖 users，與帳號刪除鎖序相反；
-- 清單 / 儲值單只是營運查找用的參照）。不存 IP（IP 只用於限流，只存 share_views）。
-- 保留期：closed 且 handled_at 超過 365 天者由 ratelimit::cleanup 刪除；帳號刪除時 account.rs 立即刪該使用者的工單。
CREATE TABLE support_requests (
  id            uuid        PRIMARY KEY DEFAULT uuid_v7(),
  user_id       uuid,
  contact_email text,
  category      text        NOT NULL CHECK (category IN ('account', 'wishlist', 'payment', 'wallet', 'other')),
  wishlist_id   uuid,
  topup_id      uuid,
  body          text        NOT NULL CHECK (char_length(body) BETWEEN 1 AND 2000),
  status        text        NOT NULL DEFAULT 'open' CHECK (status IN ('open', 'closed')),
  reply         text        CHECK (char_length(reply) <= 2000),
  handled_by    uuid,
  handled_at    timestamptz,
  created_at    timestamptz NOT NULL DEFAULT now(),
  CHECK ((user_id IS NULL) <> (contact_email IS NULL)),
  CHECK (contact_email IS NULL OR (wishlist_id IS NULL AND topup_id IS NULL))
);
CREATE INDEX support_requests_status_idx ON support_requests (status, created_at);
CREATE INDEX support_requests_user_idx ON support_requests (user_id, created_at DESC) WHERE user_id IS NOT NULL;
CREATE INDEX support_requests_email_open_idx ON support_requests (contact_email) WHERE status = 'open' AND contact_email IS NOT NULL;
