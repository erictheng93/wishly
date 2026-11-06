-- 營運角色（可多選）：admin＝全部權限＋營運人員管理 / 系統旗標 / MFA 重設 / 帳號復原；
-- finance＝金流、儲值、退款、錢包、訂單；moderator＝檢舉、下架、組織 / 許可審核、救災事件、品牌商品；
-- support＝只有所有營運人員共用的唯讀查詢（使用者、清單、稽核、統計）。守衛在 admin.rs::Staff<R>。
ALTER TABLE users ADD COLUMN staff_roles text[] NOT NULL DEFAULT '{}'
  CHECK (staff_roles <@ ARRAY['admin', 'finance', 'moderator', 'support']::text[]);

-- 既有營運人員原本就有全部權限，維持為 admin
UPDATE users SET staff_roles = '{admin}' WHERE is_staff;

-- is_staff 由 staff_roles 推導（有任一角色＝營運人員），保留欄位給既有查詢。
-- 只改 is_staff 的舊寫法（手動 SQL 設第一位 staff、測試 / QA 腳本）：設 true 且沒有角色 → admin；設 false → 清空角色。
CREATE FUNCTION users_sync_staff() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF TG_OP = 'UPDATE' AND NEW.staff_roles IS DISTINCT FROM OLD.staff_roles
     OR TG_OP = 'INSERT' AND cardinality(NEW.staff_roles) > 0 THEN
    NEW.is_staff := cardinality(NEW.staff_roles) > 0;
  ELSIF NEW.is_staff AND cardinality(NEW.staff_roles) = 0 THEN
    NEW.staff_roles := '{admin}';
  ELSIF NOT NEW.is_staff THEN
    NEW.staff_roles := '{}';
  END IF;
  RETURN NEW;
END $$;
CREATE TRIGGER users_sync_staff BEFORE INSERT OR UPDATE OF is_staff, staff_roles ON users
  FOR EACH ROW EXECUTE FUNCTION users_sync_staff();
ALTER TABLE users ADD CONSTRAINT users_staff_roles_match CHECK (is_staff = (cardinality(staff_roles) > 0));

-- session 快取（0065）也要在角色變動時失效
DROP TRIGGER users_cache_inval ON users;
CREATE TRIGGER users_cache_inval AFTER UPDATE ON users FOR EACH ROW
  WHEN (OLD.blocked_at IS DISTINCT FROM NEW.blocked_at
     OR OLD.deleted_at IS DISTINCT FROM NEW.deleted_at
     OR OLD.is_staff IS DISTINCT FROM NEW.is_staff
     OR OLD.staff_roles IS DISTINCT FROM NEW.staff_roles
     OR OLD.totp_enabled_at IS DISTINCT FROM NEW.totp_enabled_at)
  EXECUTE FUNCTION session_cache_invalidate();
