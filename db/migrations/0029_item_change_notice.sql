-- Q13：建立者修改品項內容時提醒捐助者 / 認捐者
-- 捐助者可見內容最後一次被建立者修改的時間（排序 / 圖片處理狀態不算）
ALTER TABLE wishlist_items ADD COLUMN content_changed_at timestamptz;
-- 捐助者最後一次在「我的捐助」按「知道了」的時間
ALTER TABLE claims ADD COLUMN item_change_seen_at timestamptz;
-- 連續修改合併成一封：同一收件者、同一品項只留一筆待寄
CREATE UNIQUE INDEX notifications_item_changed_uk
  ON notifications ((COALESCE(user_id, guest_id)), kind, (payload->>'item_id'))
  WHERE kind = 'claim.item_changed' AND status = 'pending';
