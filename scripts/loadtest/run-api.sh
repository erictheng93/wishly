#!/usr/bin/env bash
# 在模擬的低規 VPS（cgroup v1：/vps，1 vCPU、1 GB RAM，與 DB 容器共用）裡啟動 release 版 API。
#   scripts/loadtest/run-api.sh [binary]
# 事前準備見 scripts/loadtest/README.md。
set -euo pipefail
BIN=${1:-$(dirname "$0")/../../apps/api/target/release/wishly-api}
LOG=${LOG:-/tmp/wishly-api.log}
export APP_ENV=dev TRUSTED_PROXY=cloudflare RUST_LOG=${RUST_LOG:-warn}
# 預設走 Unix socket（DB 容器把 /var/run/postgresql 掛到 PGSOCK）；同機部署時比 TCP 經 Docker bridge 省下大量核心開銷
PGSOCK=${PGSOCK:-/tmp/wishly-pgsock}
export DATABASE_URL=${DATABASE_URL:-postgres://wishly:wishly@localhost/wishly?host=$PGSOCK}
export DB_POOL_MAX=${DB_POOL_MAX:-5}
export SMTP_HOST=127.0.0.1 SMTP_PORT=1025
pkill -f 'target/release/wishly-api' 2>/dev/null || true
sleep 0.5
# 先把自己放進 cgroup 再 exec：程式啟動時就只看得到 1 顆 CPU（tokio 依此決定 worker 數），與真正的 1 vCPU 主機一致
sh -c 'for c in cpu cpuacct cpuset memory; do
         [ -d /sys/fs/cgroup/$c/vps ] && echo $$ >/sys/fs/cgroup/$c/vps/cgroup.procs
       done; exec "$0"' "$BIN" >"$LOG" 2>&1 &
pid=$!
for _ in $(seq 1 60); do curl -fsS http://127.0.0.1:8080/healthz >/dev/null 2>&1 && break; sleep 0.5; done
echo "api pid=$pid log=$LOG"
