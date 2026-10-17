-- 每日活躍：一人一天一列（台北日），供 DAU 歷史與留存；只存日期，不存時間/IP/裝置
CREATE TABLE user_activity_days (
  user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  day     date NOT NULL,
  PRIMARY KEY (user_id, day)
);
CREATE INDEX user_activity_days_day_idx ON user_activity_days (day);

-- 回填：由現有 sessions 的建立與最後使用時間推得（未刪除使用者）
INSERT INTO user_activity_days (user_id, day)
SELECT s.user_id, (t AT TIME ZONE 'Asia/Taipei')::date
FROM sessions s JOIN users u ON u.id = s.user_id AND u.deleted_at IS NULL
CROSS JOIN LATERAL (VALUES (s.created_at), (s.last_seen_at)) v(t)
ON CONFLICT DO NOTHING;
