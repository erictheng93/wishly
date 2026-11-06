-- 精選商品可標興趣（詞彙見 interests.rs），供送禮靈感依興趣挑選
ALTER TABLE catalog_products ADD COLUMN interests text[] NOT NULL DEFAULT '{}';
