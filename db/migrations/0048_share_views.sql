-- 分享頁瀏覽紀錄：只記未登入訪客，只增不改；依產品決定永久保存（清理工作不可刪此表）
CREATE TABLE share_views (
  id            uuid PRIMARY KEY DEFAULT uuid_v7(),
  wishlist_id   uuid REFERENCES wishlists(id) ON DELETE SET NULL,
  visitor_id    uuid,
  ip            inet NOT NULL,
  country       text,
  city          text,
  device        text NOT NULL CHECK (device IN ('mobile', 'tablet', 'desktop')),
  user_agent    text NOT NULL,
  referrer_host text,
  created_at    timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX share_views_created_at_idx ON share_views (created_at);
CREATE INDEX share_views_wishlist_idx ON share_views (wishlist_id);
