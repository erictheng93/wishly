-- session 驗證快取的失效訊號（session.rs::cache）：任何會讓「已快取的成功驗證」失效的變更，
-- 都在 commit 時 pg_notify('session_inval')，各 API 實例的 listener 收到就清空快取。
-- 做在 DB 層而不是在程式各處呼叫：撤銷 session 的路徑有十幾處（登出、撤銷裝置、改密碼、停權、刪帳…），
-- 也涵蓋手動 SQL 與多實例。同一交易內相同的通知會被 Postgres 合併成一則。
-- 刻意不觸發：last_seen_at 與「延長」expires_at（每個使用中的 session 每 5 分鐘一次，觸發會讓快取形同虛設）。
CREATE FUNCTION session_cache_invalidate() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  PERFORM pg_notify('session_inval', '');
  RETURN NULL;
END $$;

CREATE TRIGGER sessions_cache_inval_upd AFTER UPDATE ON sessions FOR EACH ROW
  WHEN (OLD.revoked_at IS DISTINCT FROM NEW.revoked_at
     OR OLD.mfa_at IS DISTINCT FROM NEW.mfa_at
     OR NEW.expires_at < OLD.expires_at
     OR OLD.user_id IS DISTINCT FROM NEW.user_id
     OR OLD.token_hash IS DISTINCT FROM NEW.token_hash)
  EXECUTE FUNCTION session_cache_invalidate();
CREATE TRIGGER sessions_cache_inval_del AFTER DELETE ON sessions FOR EACH ROW
  EXECUTE FUNCTION session_cache_invalidate();

CREATE TRIGGER users_cache_inval AFTER UPDATE ON users FOR EACH ROW
  WHEN (OLD.blocked_at IS DISTINCT FROM NEW.blocked_at
     OR OLD.deleted_at IS DISTINCT FROM NEW.deleted_at
     OR OLD.is_staff IS DISTINCT FROM NEW.is_staff
     OR OLD.totp_enabled_at IS DISTINCT FROM NEW.totp_enabled_at)
  EXECUTE FUNCTION session_cache_invalidate();
CREATE TRIGGER users_cache_inval_del AFTER DELETE ON users FOR EACH ROW
  EXECUTE FUNCTION session_cache_invalidate();

-- staff_mfa_required 影響 CurrentUser.mfa_required
CREATE TRIGGER system_flags_cache_inval AFTER INSERT OR UPDATE ON system_flags FOR EACH ROW
  WHEN (NEW.key = 'staff_mfa_required')
  EXECUTE FUNCTION session_cache_invalidate();
CREATE TRIGGER system_flags_cache_inval_del AFTER DELETE ON system_flags FOR EACH ROW
  WHEN (OLD.key = 'staff_mfa_required')
  EXECUTE FUNCTION session_cache_invalidate();
