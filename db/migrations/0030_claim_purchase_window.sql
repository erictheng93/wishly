-- Q12：認領購買期限。捐助者可延長一次（7 天），記錄延長時間
ALTER TABLE claims ADD COLUMN extended_at timestamptz;
-- 既有個人 / 婚禮清單套預設 7 天；relief 保留原設定。
-- 刻意不回填既有 claims.expires_at：避免上線當下大量釋出，只影響之後的新認領
UPDATE wishlists SET claim_ttl_hours = 168 WHERE claim_ttl_hours IS NULL AND type IN ('personal', 'registry');
