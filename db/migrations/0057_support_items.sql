ALTER TABLE wishlist_items
  ADD COLUMN support_wishlist_id uuid REFERENCES wishlists(id) ON DELETE SET NULL,
  ADD COLUMN support_notified_at timestamptz,
  ADD CONSTRAINT items_support_shape CHECK (funding_mode = 'support' OR (support_wishlist_id IS NULL AND support_notified_at IS NULL));
-- 刻意不加外鍵：外鍵檢查會對發起者的品項加 KEY SHARE 鎖，把它捲進對方清單的鎖序（同 0050）
ALTER TABLE contributions ADD COLUMN via_item_id uuid;
ALTER TABLE claims ADD COLUMN via_item_id uuid;
CREATE INDEX contributions_via_idx ON contributions (via_item_id) WHERE via_item_id IS NOT NULL;
CREATE INDEX claims_via_idx ON claims (via_item_id) WHERE via_item_id IS NOT NULL;
