-- 組織撤銷後 7 天內可恢復認證：記下撤銷時間，與撤銷當下被降為草稿的據點（無回填，舊撤銷不可恢復）
ALTER TABLE organizations ADD COLUMN revoked_at timestamptz;
ALTER TABLE wishlists ADD COLUMN revoke_paused_at timestamptz;
CREATE INDEX wishlists_revoke_paused_idx ON wishlists (org_id) WHERE revoke_paused_at IS NOT NULL;
