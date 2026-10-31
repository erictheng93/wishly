-- Web Push 訂閱（D67）：每台裝置一列，endpoint 全域唯一（同一瀏覽器換帳號登入時歸屬改到新使用者）
CREATE TABLE push_subscriptions (
  id         uuid        PRIMARY KEY DEFAULT gen_random_uuid(),
  user_id    uuid        NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  endpoint   text        NOT NULL UNIQUE,
  p256dh     text        NOT NULL,
  auth       text        NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX push_subscriptions_user ON push_subscriptions (user_id, created_at DESC);
