-- 發佈圍欄：publish_checked 寫 origin 前記下期限。程序在發佈與複查之間崩潰時，
-- drain 要在期限過後再刪一次才標記 purged_at，晚落地的寫入不會留在 bucket。
ALTER TABLE user_media ADD COLUMN publishing_until timestamptz;
