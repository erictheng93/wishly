-- 效能第二批索引（docs/perf-vps-2026-11.md）：以下查詢原本都是整表掃描。
-- 事件頁 / 事件列表 / 事件編輯：WHERE w.event_id = $1
CREATE INDEX wishlists_event_idx ON wishlists (event_id) WHERE event_id IS NOT NULL;
-- 組織公開頁、據點列表、組織稽核：WHERE w.org_id = $1
CREATE INDEX wishlists_org_idx ON wishlists (org_id) WHERE org_id IS NOT NULL;
-- 實收：每個品項 SUM(qty)（ICOLS 的 qty_received）與登記頁的最近幾筆；也供刪品項時的 FK cascade
CREATE INDEX receipts_item_idx ON receipts (item_id, created_at DESC, id DESC);
-- 刪據點時的 FK cascade
CREATE INDEX receipts_wishlist_idx ON receipts (wishlist_id);
