-- 新手導覽（D66）：完成或跳過時寫入；NULL = 導覽進行中。各步驟是否完成由既有資料推導（onboarding.rs），不另存。
ALTER TABLE users ADD COLUMN onboarding_done_at timestamptz;

-- 上線前就有的帳號不再跳導覽（設定頁可重新開始）
UPDATE users SET onboarding_done_at = now();
