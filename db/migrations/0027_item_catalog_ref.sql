-- 靈感頁精選商品代號；P2 接 catalog_products 時沿用
ALTER TABLE wishlist_items ADD COLUMN catalog_ref text;
CREATE INDEX wishlist_items_catalog_ref_idx ON wishlist_items (catalog_ref) WHERE catalog_ref IS NOT NULL AND deleted_at IS NULL;
