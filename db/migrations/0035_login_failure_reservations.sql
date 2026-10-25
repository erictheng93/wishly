-- 登入先預約一個嘗試名額（pending_until 有值），驗證完才定案：密碼錯誤改為 NULL（確定的失敗），
-- 成功或系統錯誤刪除。請求中途被取消時預約到期即不再計入鎖定；後台分析只算 pending_until IS NULL。
ALTER TABLE login_failures ADD COLUMN pending_until timestamptz;
