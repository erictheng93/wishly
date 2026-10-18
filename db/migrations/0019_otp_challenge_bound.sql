-- 匿名端點（otp/request、register、password/reset/request）發出的驗證碼會把 challenge_id 回給請求者，
-- 驗證時必須帶同一個 id；不帶 id 的舊路徑只能驗未綁定的挑戰（後台代寄的重設信、登入中綁定 Email），
-- 避免他人不帶 id 亂猜而燒掉本人那張驗證碼的嘗試次數。
ALTER TABLE otp_challenges ADD COLUMN bound boolean NOT NULL DEFAULT false;
