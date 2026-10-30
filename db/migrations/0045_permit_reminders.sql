-- 勸募許可到期前提醒（14 / 7 天）：記錄已寄過的最小門檻，條件式 UPDATE 保證每個門檻只排一次（多實例、重啟安全）
ALTER TABLE org_permits ADD COLUMN reminded_days smallint;
