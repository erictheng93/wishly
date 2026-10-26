-- 好友邀請連結可附帶一份 friends / selected 清單：接受後成為好友並直接進該清單
ALTER TABLE friend_invites ADD COLUMN wishlist_id uuid REFERENCES wishlists(id) ON DELETE CASCADE;
-- 依 (user_id, wishlist_id) 撤銷有效邀請
CREATE INDEX friend_invites_active_idx ON friend_invites (user_id, wishlist_id) WHERE revoked_at IS NULL;
