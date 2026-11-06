-- 帳號層級摯友名單取代每份清單的 selected 名單（wishlist_allowed_users）。
-- 名單為空（或勾的人已非好友）的 selected 清單改 private，避免變成對全部摯友意外公開。
UPDATE wishlists w SET visibility = 'private'
 WHERE w.visibility = 'selected' AND w.org_id IS NULL
   AND NOT EXISTS (SELECT 1 FROM wishlist_allowed_users a
                    WHERE a.wishlist_id = w.id
                      AND EXISTS (SELECT 1 FROM friendships f
                                   WHERE f.user_a = LEAST(w.owner_id, a.user_id) AND f.user_b = GREATEST(w.owner_id, a.user_id)));
-- 組織清單本來就不該有 selected，預期 0 列
UPDATE wishlists SET visibility = 'private' WHERE visibility = 'selected' AND org_id IS NOT NULL;

CREATE TABLE close_friends (
  owner_id   uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  friend_id  uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  created_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (owner_id, friend_id)
);
CREATE INDEX close_friends_friend_idx ON close_friends(friend_id);

INSERT INTO close_friends (owner_id, friend_id)
SELECT DISTINCT w.owner_id, a.user_id
  FROM wishlist_allowed_users a JOIN wishlists w ON w.id = a.wishlist_id
 WHERE w.org_id IS NULL
   AND EXISTS (SELECT 1 FROM friendships f
                WHERE f.user_a = LEAST(w.owner_id, a.user_id) AND f.user_b = GREATEST(w.owner_id, a.user_id))
ON CONFLICT DO NOTHING;

DROP TABLE wishlist_allowed_users;
