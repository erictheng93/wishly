#!/usr/bin/env bash
# 安裝主機防火牆並開機自動套用；Docker 重啟時一併重新套用。以 root 執行，需與 wishly-firewall.sh 放同一目錄。
# 部署方式與現況見 docs/deploy-backend-checklist.md §2.3。
set -euo pipefail
cd "$(dirname "$0")"
install -m 755 wishly-firewall.sh /usr/local/sbin/wishly-firewall.sh

cat >/etc/systemd/system/wishly-firewall.service <<'EOF'
[Unit]
Description=Wishly host firewall (second layer behind Linode Cloud Firewall)
After=docker.service
Wants=docker.service
PartOf=docker.service

[Service]
Type=oneshot
RemainAfterExit=yes
ExecStart=/usr/local/sbin/wishly-firewall.sh on
ExecStop=/usr/local/sbin/wishly-firewall.sh off

[Install]
WantedBy=multi-user.target docker.service
EOF
systemctl daemon-reload
systemctl enable --now wishly-firewall.service

echo "== DOCKER-USER";  iptables -S DOCKER-USER;  ip6tables -S DOCKER-USER 2>/dev/null || true
echo "== INPUT (eth0)"; iptables -S INPUT | grep eth0; ip6tables -S INPUT | grep eth0
echo "== cloudflared 最近 5 行（應無 connection 錯誤）"
docker logs --since 2m "$(docker ps -q -f name=wishly-cloudflared)" 2>&1 | tail -5
