-- 使用者停權保留帳號與既有履行紀錄；解鎖不恢復已撤銷的 session。
ALTER TABLE users ADD COLUMN blocked_at timestamptz;
