-- 清單自動結束時間（選填，預設 NULL = 不設期限）；到期由背景 job 將 active 轉 closed，創建者仍可重開。
ALTER TABLE wishlists ADD COLUMN closes_at timestamptz;
CREATE INDEX wishlists_closes_at_idx ON wishlists (closes_at) WHERE status = 'active' AND closes_at IS NOT NULL;
