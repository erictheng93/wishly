-- 綁定登入方式撞到另一個帳號時，只有「空帳號」能被併入後刪除（identities.rs::absorb）。
-- 逐一檢查所有指向 users 的外鍵：日後新增的表自動算作「有資料」，不必回頭維護清單。
-- 白名單只放登入 / 系統雜項；錢包須零餘額且沒有任何帳本紀錄。
CREATE FUNCTION user_is_blank(uid uuid) RETURNS boolean LANGUAGE plpgsql STABLE AS $$
DECLARE
  r record;
  hit boolean;
BEGIN
  IF NOT EXISTS (SELECT 1 FROM users WHERE id = uid AND NOT is_staff AND blocked_at IS NULL AND deleted_at IS NULL) THEN
    RETURN false;
  END IF;
  FOR r IN
    SELECT c.conrelid::regclass AS tbl, a.attname AS col
      FROM pg_constraint c
      JOIN pg_attribute a ON a.attrelid = c.conrelid AND a.attnum = ANY (c.conkey)
     WHERE c.contype = 'f' AND c.confrelid = 'users'::regclass
       AND c.conrelid NOT IN ('auth_identities'::regclass, 'sessions'::regclass, 'notifications'::regclass,
                              'user_activity_days'::regclass, 'native_auth_codes'::regclass,
                              'otp_challenges'::regclass, 'user_media'::regclass, 'point_wallets'::regclass)
  LOOP
    EXECUTE format('SELECT EXISTS (SELECT 1 FROM %s WHERE %I = $1)', r.tbl, r.col) INTO hit USING uid;
    IF hit THEN
      RETURN false;
    END IF;
  END LOOP;
  RETURN NOT EXISTS (SELECT 1 FROM point_wallets w WHERE w.user_id = uid
                       AND (w.balance <> 0 OR w.status <> 'active' OR EXISTS (SELECT 1 FROM point_ledger l WHERE l.wallet_id = w.id)));
END $$;
