-- 原生殼綁定第三方帳號：系統瀏覽器帶不了 Bearer，先由 App 以已登入身分換一張一次性 ticket（purpose='link'），
-- 綁定 app_challenge 與發起的 session，瀏覽器帶 ticket 進 start。與登入碼（purpose='login'）分開兌換。
ALTER TABLE native_auth_codes
  ADD COLUMN purpose text NOT NULL DEFAULT 'login' CHECK (purpose IN ('login','link')),
  ADD COLUMN session_hash bytea;
