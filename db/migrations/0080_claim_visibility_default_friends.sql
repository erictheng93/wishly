-- 新帳號的捐助預設可見度改為 friends（原 private 會讓「顯示送禮者暱稱」全顯示匿名）；既有帳號不動，無法分辨是否刻意選私人
ALTER TABLE users ALTER COLUMN default_claim_visibility SET DEFAULT 'friends';
