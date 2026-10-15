-- 原生殼 OAuth：系統瀏覽器完成登入後以 deep link 帶回的一次性碼（60 秒、單次使用，只存雜湊）。
-- challenge = base64url(SHA-256(app_verifier))；兌換時須出示 verifier，避免被別的 App 搶註冊的 scheme 攔走。
CREATE TABLE native_auth_codes (
  code_hash  bytea PRIMARY KEY,
  user_id    uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  challenge  text NOT NULL,
  expires_at timestamptz NOT NULL
);
