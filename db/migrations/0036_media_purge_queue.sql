-- 圖片刪除改為持久待辦：要求退役（retire_requested_at）與 origin 已刪（purged_at）都記在 DB，
-- 程序重啟後由背景 worker 接著做完，不再只靠記憶體中的重試。
ALTER TABLE user_media
  ADD COLUMN retire_requested_at timestamptz,
  ADD COLUMN purged_at timestamptz;
CREATE INDEX user_media_pending_idx ON user_media (object_key)
  WHERE (retire_requested_at IS NOT NULL AND deleted_at IS NULL) OR (deleted_at IS NOT NULL AND purged_at IS NULL);
