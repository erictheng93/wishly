-- 個人頁「重要日子」：本人自訂（紀念日、孩子生日…），好友在 7 天前收到站內通知
CREATE TABLE user_dates (
  id         uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  user_id    uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  title      text NOT NULL CHECK (char_length(title) BETWEEN 1 AND 30),
  month_day  text NOT NULL CHECK (month_day ~ '^(0[1-9]|1[0-2])-(0[1-9]|[12][0-9]|3[01])$'),   -- 'MM-DD'；合法性（如 02-30）由 API 檢查
  year       smallint CHECK (year BETWEEN 1900 AND 2100),
  recurring  boolean NOT NULL DEFAULT true,
  visibility profile_vis NOT NULL DEFAULT 'friends',
  created_at timestamptz NOT NULL DEFAULT now(),
  CHECK (recurring OR year IS NOT NULL)
);
CREATE INDEX user_dates_user_idx ON user_dates(user_id);

-- 每個日子每年只通知一次（冪等）
CREATE TABLE user_date_sent (
  date_id uuid NOT NULL REFERENCES user_dates(id) ON DELETE CASCADE,
  year    smallint NOT NULL,
  PRIMARY KEY (date_id, year)
);
