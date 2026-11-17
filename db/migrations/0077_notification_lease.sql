-- 通知租約：領取的 worker 持有 lease_id 到 lease_until，其他 worker 的合併不可動這些列（避免滾動部署時重複寄送）
ALTER TABLE notifications ADD COLUMN lease_id uuid, ADD COLUMN lease_until timestamptz;
