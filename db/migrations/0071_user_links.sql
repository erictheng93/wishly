-- 個人頁外部連結（最多 10 筆，整組覆寫）
CREATE TABLE user_links (
  id         uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  user_id    uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  title      text NOT NULL CHECK (char_length(title) BETWEEN 1 AND 40),
  url        text NOT NULL CHECK (char_length(url) <= 500 AND url LIKE 'https://%'),
  position   smallint NOT NULL,
  hidden     boolean NOT NULL DEFAULT false,
  created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX user_links_user_pos_idx ON user_links (user_id, position);
