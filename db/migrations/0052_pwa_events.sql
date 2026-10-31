-- PWA 安裝事件（pwa_install_prompted / pwa_installed）：匿名計數，不存使用者、IP、UA。
CREATE TABLE pwa_events (
  id         bigserial PRIMARY KEY,
  event      text NOT NULL CHECK (event IN ('pwa_install_prompted', 'pwa_installed')),
  platform   text NOT NULL CHECK (platform IN ('ios', 'android')),
  context    text NOT NULL CHECK (context IN ('claim', 'fund', 'appinstalled', 'first_run')),
  created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX pwa_events_created_at_idx ON pwa_events (created_at);
