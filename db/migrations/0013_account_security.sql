-- 帳號安全：營運人員 TOTP 兩步驟驗證、session 記錄第二因素通過時間。
-- totp_secret 以 sealed.rs（SHIPPING_ENC_KEY）加密；金鑰遺失時由其他營運人員重設（DELETE /admin/users/{id}/mfa）。
ALTER TABLE users
  ADD COLUMN totp_secret     bytea,          -- 設定中（未啟用）也會有值；重新 setup 會覆寫
  ADD COLUMN totp_enabled_at timestamptz,
  ADD COLUMN totp_last_step  bigint;         -- 最後一次接受的 30 秒時間步，防同一碼重放

-- 此 session 通過 TOTP 的時間；已啟用 TOTP 的營運人員，Staff 守衛要求非空。
ALTER TABLE sessions ADD COLUMN mfa_at timestamptz;
