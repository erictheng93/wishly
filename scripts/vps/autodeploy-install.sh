#!/usr/bin/env bash
# 安裝 GHCR latest 輪詢自動部署（每分鐘）。以 root 執行，需與 wishly-autodeploy.sh 放同一目錄。
# 先建立 /etc/wishly-autodeploy.env（見 docs/deploy-runbook.md §1「自動部署」）。
set -euo pipefail
cd "$(dirname "$0")"
env=/etc/wishly-autodeploy.env
[ -f "$env" ] || { echo "缺少 $env，請先依 docs/deploy-runbook.md §1 建立" >&2; exit 1; }
chown root:root "$env"
chmod 600 "$env"
install -m 755 wishly-autodeploy.sh /usr/local/sbin/wishly-autodeploy.sh

cat >/etc/systemd/system/wishly-autodeploy.service <<'UNIT'
[Unit]
Description=Redeploy Dokploy api when ghcr.io/erictheng93/wishly-api:latest changes
After=network-online.target docker.service
Wants=network-online.target

[Service]
Type=oneshot
ExecStart=/usr/local/sbin/wishly-autodeploy.sh
UNIT

cat >/etc/systemd/system/wishly-autodeploy.timer <<'UNIT'
[Unit]
Description=Poll GHCR for a new wishly-api:latest every minute

[Timer]
OnBootSec=2min
OnUnitActiveSec=60s
AccuracySec=5s

[Install]
WantedBy=timers.target
UNIT

systemctl daemon-reload
/usr/local/sbin/wishly-autodeploy.sh # 第一次只記錄目前的 latest；憑證錯誤會在這裡就失敗
systemctl enable --now wishly-autodeploy.timer
systemctl list-timers wishly-autodeploy.timer --no-pager
