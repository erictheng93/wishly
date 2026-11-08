-- 綁定 / 解除登入方式須近期驗證：session 建立後 10 分鐘內，或輸入密碼 / Email 驗證碼後 10 分鐘內。
ALTER TABLE sessions ADD COLUMN reauth_at timestamptz;
ALTER TABLE otp_challenges DROP CONSTRAINT otp_challenges_purpose_check;
ALTER TABLE otp_challenges
  ADD CONSTRAINT otp_challenges_purpose_check CHECK (purpose IN ('login','register','reset','link','reauth'));
ALTER TABLE otp_challenges DROP CONSTRAINT otp_challenges_link_scope;
ALTER TABLE otp_challenges
  ADD CONSTRAINT otp_challenges_link_scope CHECK (
    (purpose IN ('link','reauth') AND link_user_id IS NOT NULL AND link_session_hash IS NOT NULL)
    OR (purpose NOT IN ('link','reauth') AND link_user_id IS NULL AND link_session_hash IS NULL));
