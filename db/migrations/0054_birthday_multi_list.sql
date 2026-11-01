-- 生日通知可開多份清單（D65 修訂）：不同好友圈看不同生日清單。
DROP INDEX wishlists_birthday_notify_uk;

-- 「哪一年已寄過」改記在每份清單。獨立成表：寫 wishlists 會觸發 updated_at，讓壽星正在編輯的 PATCH 誤判 409。
CREATE TABLE wishlist_birthday_sent (
  wishlist_id uuid     NOT NULL REFERENCES wishlists(id) ON DELETE CASCADE,
  year        smallint NOT NULL,
  PRIMARY KEY (wishlist_id, year)
);

-- 今年已寄過的清單不重寄。
INSERT INTO wishlist_birthday_sent (wishlist_id, year)
SELECT w.id, u.birthday_notified_year
  FROM wishlists w JOIN users u ON u.id = w.owner_id
 WHERE w.birthday_notify AND w.deleted_at IS NULL AND u.birthday_notified_year IS NOT NULL;

ALTER TABLE users DROP COLUMN birthday_notified_year;
