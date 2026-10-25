-- 成功登入 / 重設或變更密碼改為標記 cleared_at，不再刪除：鎖定只看未清除的紀錄，
-- 後台分析（trust.login_failures）才能算到「失敗後又登入成功」的那幾次。
ALTER TABLE login_failures ADD COLUMN cleared_at timestamptz;
-- 背景清理（保留 365 天）與分析區間計數都只依 created_at 過濾
CREATE INDEX login_failures_created_idx ON login_failures (created_at);
