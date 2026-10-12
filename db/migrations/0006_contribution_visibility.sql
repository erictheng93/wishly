-- 認捐逐筆可見度：回填目前有效的帳號預設，之後改預設不回溯。
ALTER TABLE contributions ADD COLUMN visibility share_level;
-- SQLx 在同一交易中執行 migration；僅回填期間停用時間戳 trigger，保留原 updated_at。
ALTER TABLE contributions DISABLE TRIGGER contributions_set_updated_at;
UPDATE contributions c SET visibility = u.default_claim_visibility
  FROM users u WHERE u.id = c.user_id;
ALTER TABLE contributions ENABLE TRIGGER contributions_set_updated_at;
ALTER TABLE contributions ALTER COLUMN visibility SET NOT NULL;

CREATE FUNCTION contributions_default_visibility() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  IF NEW.visibility IS NULL THEN
    SELECT default_claim_visibility INTO NEW.visibility FROM users WHERE id = NEW.user_id;
  END IF;
  RETURN NEW;
END $$;
CREATE TRIGGER contributions_default_visibility BEFORE INSERT OR UPDATE OF user_id ON contributions
  FOR EACH ROW EXECUTE FUNCTION contributions_default_visibility();
