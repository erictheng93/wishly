-- 事件下架時被暫停的上線據點：記下暫停時間，取消下架時只恢復有標記者（下架前已暫停 / 關閉的不動）。
-- 據點手動改 site_status、改 status 或換事件、事件被合併時清除；語意比照 0063 的 revoke_paused_at
ALTER TABLE wishlists ADD COLUMN event_paused_at timestamptz;
CREATE INDEX wishlists_event_paused_idx ON wishlists (event_id) WHERE event_paused_at IS NOT NULL;
