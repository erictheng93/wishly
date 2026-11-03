-- 稽核紀錄只增不改：UPDATE 一律拒絕；DELETE 只有保留期清理工作能做
-- （在自己的交易內 set_config('wishly.audit_purge','on',true)）；TRUNCATE 一律拒絕。
-- 注意：資料庫超級使用者仍可 ALTER TABLE ... DISABLE TRIGGER 繞過，
-- 這裡防的是程式錯誤與隨手改資料，不是惡意 DBA。
CREATE FUNCTION audit_logs_guard() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF TG_OP = 'DELETE' AND current_setting('wishly.audit_purge', true) = 'on' THEN
    RETURN OLD;
  END IF;
  RAISE EXCEPTION 'audit_logs 為只增不改（%）', TG_OP USING ERRCODE = 'insufficient_privilege';
END $$;

CREATE TRIGGER audit_logs_no_mutate BEFORE UPDATE OR DELETE ON audit_logs
  FOR EACH ROW EXECUTE FUNCTION audit_logs_guard();
CREATE TRIGGER audit_logs_no_truncate BEFORE TRUNCATE ON audit_logs
  FOR EACH STATEMENT EXECUTE FUNCTION audit_logs_guard();

-- 請求脈絡：由 middleware 帶入（背景工作沒有請求，維持 NULL）
ALTER TABLE audit_logs ADD COLUMN request_id uuid, ADD COLUMN user_agent text;
CREATE INDEX audit_logs_request_id ON audit_logs (request_id) WHERE request_id IS NOT NULL;

-- 保留期清理（ratelimit::cleanup 每小時）只掃內容編輯類；條件須與清理的 IN 清單一致才用得到
CREATE INDEX audit_logs_content_edit_created ON audit_logs (created_at)
  WHERE action IN ('wishlist.create','wishlist.update','item.create','item.update','item.reorder','wishlist.allowed_users.update');
