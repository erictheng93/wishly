-- 可見性精簡：拿掉「知道連結（link）」與「密碼保護（password）」，預設改為僅好友。
-- 既有 link / password 清單收緊為 friends（寧可少給，不要意外變公開）。enum 值無法移除，改用 CHECK 擋掉。
ALTER TABLE wishlists DROP CONSTRAINT wishlists_password_has_hash;
UPDATE wishlists SET visibility = 'friends' WHERE visibility::text IN ('link', 'password');
ALTER TABLE wishlists DROP COLUMN access_password_hash;
ALTER TABLE wishlists ALTER COLUMN visibility SET DEFAULT 'friends';
ALTER TABLE wishlists ADD CONSTRAINT wishlists_visibility_supported CHECK (visibility::text NOT IN ('link', 'password'));
