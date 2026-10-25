-- Keep image history after replacement; a retired key can never be published again.
CREATE TABLE user_media (
  user_id uuid NOT NULL REFERENCES users(id),
  object_key text NOT NULL,
  deleted_at timestamptz,
  PRIMARY KEY (user_id, object_key)
);
CREATE INDEX user_media_key_idx ON user_media (object_key);

INSERT INTO user_media (user_id, object_key)
SELECT owner_id, cover_image_key FROM wishlists WHERE cover_image_key IS NOT NULL
UNION
SELECT w.owner_id, i.image_key FROM wishlist_items i JOIN wishlists w ON w.id=i.wishlist_id WHERE i.image_key IS NOT NULL
UNION
SELECT id, avatar_key FROM users WHERE avatar_key LIKE 'avatars/%'
ON CONFLICT DO NOTHING;
