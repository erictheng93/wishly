-- 綁定 OTP 只對發起帳號與 session 有效，不能拿來登入、註冊或重設密碼。
ALTER TABLE otp_challenges DROP CONSTRAINT otp_challenges_purpose_check;
ALTER TABLE otp_challenges
  ADD CONSTRAINT otp_challenges_purpose_check CHECK (purpose IN ('login','register','reset','link')),
  ADD COLUMN link_user_id uuid REFERENCES users(id) ON DELETE CASCADE,
  ADD COLUMN link_session_hash bytea,
  ADD CONSTRAINT otp_challenges_link_scope CHECK (
    (purpose = 'link' AND link_user_id IS NOT NULL AND link_session_hash IS NOT NULL)
    OR (purpose <> 'link' AND link_user_id IS NULL AND link_session_hash IS NULL));
