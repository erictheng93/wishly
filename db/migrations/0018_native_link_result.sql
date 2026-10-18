-- 原生殼綁定第三方帳號：瀏覽器 callback 不再直接寫 auth_identities，
-- 只存一筆 60 秒單次的待確認結果（purpose='link_result'），由 App 憑 PKCE verifier + 同一個 Bearer session 呼叫 link-confirm 才真正綁定。
ALTER TABLE native_auth_codes
  DROP CONSTRAINT native_auth_codes_purpose_check,
  ADD CONSTRAINT native_auth_codes_purpose_check CHECK (purpose IN ('login','link','link_result')),
  ADD COLUMN provider text,
  ADD COLUMN provider_uid text,
  ADD COLUMN email text,
  ADD COLUMN picture text;
