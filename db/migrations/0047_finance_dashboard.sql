-- 金流後台：依 paid_at 統計的索引
CREATE INDEX topups_paid_at_idx ON topups (paid_at) WHERE paid_at IS NOT NULL;
-- 舊拒付的 updated_at 不是拒付時間：以稽核紀錄回填，否則會算到錯的區間
UPDATE topups t SET updated_at = a.at
  FROM (SELECT entity_id, max(created_at) AS at FROM audit_logs
         WHERE action = 'topup.charged_back' AND entity = 'topups' GROUP BY 1) a
 WHERE t.id = a.entity_id AND t.status = 'charged_back';
