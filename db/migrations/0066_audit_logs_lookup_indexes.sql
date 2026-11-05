-- 稽核查詢索引：後台 /admin/audit-logs 只帶 actor_id 或只帶 action 篩選時，舊索引（actor_type 開頭）用不到，
-- 只能倒掃主鍵逐列過濾；action 罕見或不存在時近乎整表掃描（100 萬筆實測 157 ms / 72 ms → 0.04 ms）。
-- 兩個索引都以 id（uuid_v7，時間序）結尾，對應 ORDER BY id DESC 的分頁。
-- 舊的 (actor_type, actor_id, created_at) 由 (actor_id, id) 取代：所有讀取端（後台、本人安全活動、匯出次數）都以 actor_id 為主。
DROP INDEX audit_logs_actor_idx;
CREATE INDEX audit_logs_actor_id_idx ON audit_logs (actor_id, id DESC) WHERE actor_id IS NOT NULL;
CREATE INDEX audit_logs_action_idx ON audit_logs (action, id DESC);
