-- 個人資料：自我介紹與生日（只存月日，不收年份；個資最少蒐集）
ALTER TABLE users
  ADD COLUMN bio                 text CHECK (char_length(bio) BETWEEN 1 AND 160),   -- 個人頁顯示；NULL = 未填
  ADD COLUMN birthday            text CHECK (birthday ~ '^(0[1-9]|1[0-2])-(0[1-9]|[12][0-9]|3[01])$'),   -- 'MM-DD'；日期合法性（如 02-30）由 API 檢查
  ADD COLUMN birthday_visibility share_level NOT NULL DEFAULT 'friends';            -- 個人頁誰看得到生日
