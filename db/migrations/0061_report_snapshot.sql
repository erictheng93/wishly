-- 檢舉當下的內容快照（清單 / 使用者事後被編輯或刪除，staff 仍能看到被檢舉時的樣子）。舊資料為 NULL。
-- 只存圖片 key（不存 URL）；內容不含地址、電話、Email、IP。
ALTER TABLE content_reports ADD COLUMN snapshot jsonb;

-- 後台顯示檢舉者累計（reporter_stats）用
CREATE INDEX content_reports_reporter_user_idx ON content_reports (reporter_user_id) WHERE reporter_user_id IS NOT NULL;
CREATE INDEX content_reports_reporter_guest_idx ON content_reports (reporter_guest_id) WHERE reporter_guest_id IS NOT NULL;
