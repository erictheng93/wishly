-- 未讀數否則得掃過使用者所有已讀通知；部分索引只含未讀列
CREATE INDEX notifications_unread_idx ON notifications (user_id) WHERE inbox_at IS NOT NULL AND read_at IS NULL;
