-- closes_at 到期前提醒（寄一次）：寄出時記時間，創建者改動 closes_at 時清空以便重新提醒。
ALTER TABLE wishlists ADD COLUMN closes_reminded_at timestamptz;
