-- 探索頁：公開且上線中的清單，依建立時間倒序
CREATE INDEX wishlists_explore_idx ON wishlists (created_at DESC, id DESC)
  WHERE visibility = 'public' AND status = 'active' AND deleted_at IS NULL;
