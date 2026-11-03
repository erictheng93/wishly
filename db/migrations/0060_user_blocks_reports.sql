-- 封鎖使用者與檢舉使用者。
-- 封鎖：blocker 封鎖 blocked，效果對雙方對稱（個人頁、受限清單、好友申請）；只存單向列，查詢時看任一方向。
CREATE TABLE user_blocks (
  blocker_id uuid        NOT NULL REFERENCES users(id),
  blocked_id uuid        NOT NULL REFERENCES users(id),
  created_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (blocker_id, blocked_id),
  CHECK (blocker_id <> blocked_id)
);
CREATE INDEX user_blocks_blocked_idx ON user_blocks (blocked_id);

-- 檢舉使用者沿用 content_reports 佇列：目標是清單或使用者，恰好其一
ALTER TABLE content_reports
  ADD COLUMN reported_user_id uuid REFERENCES users(id),
  ALTER COLUMN wishlist_id DROP NOT NULL,
  ADD CONSTRAINT reports_single_target CHECK ((wishlist_id IS NULL) <> (reported_user_id IS NULL));
CREATE INDEX content_reports_user_idx ON content_reports (reported_user_id, created_at DESC) WHERE reported_user_id IS NOT NULL;
