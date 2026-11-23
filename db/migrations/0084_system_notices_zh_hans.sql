-- 簡體中文介面：系統公告語系加入 zh-Hans
ALTER TABLE system_notices DROP CONSTRAINT system_notices_locale_check;
ALTER TABLE system_notices ADD CONSTRAINT system_notices_locale_check
  CHECK (locale IN ('zh-Hant', 'zh-Hans', 'en', 'vi', 'id'));
