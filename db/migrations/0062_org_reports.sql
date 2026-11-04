-- 檢舉公益組織：沿用 content_reports 佇列，目標改為清單 / 使用者 / 組織恰好其一。
ALTER TABLE content_reports
  ADD COLUMN reported_org_id uuid REFERENCES organizations(id),
  DROP CONSTRAINT reports_single_target,
  ADD CONSTRAINT reports_single_target CHECK (num_nonnulls(wishlist_id, reported_user_id, reported_org_id) = 1);
CREATE INDEX content_reports_org_idx ON content_reports (reported_org_id, created_at DESC) WHERE reported_org_id IS NOT NULL;
