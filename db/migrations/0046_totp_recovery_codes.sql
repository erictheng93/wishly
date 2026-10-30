-- 營運人員 TOTP 備用碼（#99）：遺失驗證器時自行救回，不必找另一位營運人員；只存 SHA-256，每碼只能用一次
CREATE TABLE totp_recovery_codes (
  user_id   uuid        NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  code_hash bytea       NOT NULL,
  used_at   timestamptz,
  PRIMARY KEY (user_id, code_hash)
);

-- 後台全面強制 MFA 的手動開關：預設關閉（只有已啟用者須驗證），大家設定好備用碼後由營運人員開啟
INSERT INTO system_flags (key, value) VALUES ('staff_mfa_required', 'false'::jsonb) ON CONFLICT (key) DO NOTHING;
