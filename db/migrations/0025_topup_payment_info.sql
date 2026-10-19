-- 儲值取號資訊（D56）：綠界 PaymentInfoURL 通知的超商代碼 / 條碼 / ATM 虛擬帳號，對外 JSON 結構見 docs/04 t-topups
ALTER TABLE topups ADD COLUMN payment_info jsonb;
