-- 一般認領的匿名選項（比照 contributions.is_anonymous）：受捐者與公開頁只看到「匿名」
ALTER TABLE claims ADD COLUMN is_anonymous boolean NOT NULL DEFAULT false;

-- 驚喜鎖定的唯一定義；event_date 為 NULL 視為鎖定（寧可多藏，不洩漏）
CREATE FUNCTION surprise_locked(surprise_mode boolean, event_date date) RETURNS boolean
  LANGUAGE sql STABLE
  AS $$ SELECT surprise_mode AND (event_date IS NULL OR now() < (event_date::timestamp AT TIME ZONE 'Asia/Taipei')) $$;

-- 不再寄「有新捐助」通知（會轟炸受捐者，也破壞驚喜）；已排程的一併取消
UPDATE notifications SET status = 'cancelled' WHERE status = 'pending' AND kind IN ('claim.created', 'claim.digest');
