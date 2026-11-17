-- 系統公告（#96）：營運後台發佈，/public/status 帶出生效中的公告供前端顯示橫幅
CREATE TYPE notice_level AS ENUM ('info', 'warning', 'critical');

CREATE TABLE system_notices (
  id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  title       text NOT NULL CHECK (char_length(title) BETWEEN 1 AND 80),
  body        text NOT NULL DEFAULT '' CHECK (char_length(body) <= 500),
  level       notice_level NOT NULL DEFAULT 'info',
  -- NULL = 所有語系
  locale      text CHECK (locale IN ('zh-Hant', 'en', 'vi', 'id')),
  starts_at   timestamptz NOT NULL DEFAULT now(),
  ends_at     timestamptz CHECK (ends_at IS NULL OR ends_at > starts_at),
  disabled_at timestamptz,
  created_by  uuid NOT NULL REFERENCES users(id),
  created_at  timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX system_notices_active_idx ON system_notices (starts_at DESC) WHERE disabled_at IS NULL;
