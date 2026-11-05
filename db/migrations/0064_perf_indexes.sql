-- 效能：分享頁 ETag 取 max(wishlist_items.updated_at) 時「刻意不排除已刪除品項」（刪品項也要讓 ETag 失效），
-- 無法用 items_wishlist_idx（部分索引，deleted_at IS NULL），每次都循序掃描整張 wishlist_items。
-- 這個完整索引也讓刪除清單時的外鍵 CASCADE 不必掃全表。
CREATE INDEX wishlist_items_wishlist_updated_idx ON wishlist_items (wishlist_id, updated_at);
