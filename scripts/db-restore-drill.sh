#!/usr/bin/env bash
# 還原演練：把 dump 還原到隔離的暫存容器（無網路）→ 對帳 → 印出耗時，結束後移除容器。
# 用法：scripts/db-restore-drill.sh <dump 檔> [對帳 SQL，預設 scripts/db-reconcile.sql]
set -euo pipefail
DUMP=${1:?用法：$0 <dump 檔> [reconcile.sql]}
SQL=${2:-$(dirname "$0")/db-reconcile.sql}
IMAGE=${PG_IMAGE:-postgis/postgis:16-3.6}
C=wishly-restore-drill-$$
start=$(date +%s)
trap 'docker rm -f "$C" >/dev/null 2>&1 || true' EXIT

docker run -d --name "$C" --network none -e POSTGRES_USER=wishly -e POSTGRES_PASSWORD=drill -e POSTGRES_DB=wishly "$IMAGE" >/dev/null
# 初始化期間的暫存伺服器只聽 unix socket，用 TCP 探測才不會提早通過
for _ in $(seq 120); do docker exec "$C" pg_isready -q -h 127.0.0.1 -U wishly && break; sleep 1; done
docker exec "$C" pg_isready -q -h 127.0.0.1 -U wishly

# postgis 映像會在 POSTGRES_DB 預裝擴充，與 dump 衝突；從 template0 開乾淨的庫
docker exec "$C" createdb -U wishly -T template0 restored
docker exec -i "$C" pg_restore -U wishly -d restored --no-owner --exit-on-error <"$DUMP"
docker exec -i "$C" psql -U wishly -d restored -X <"$SQL"
echo "還原演練通過：$(basename "$DUMP")，耗時 $(($(date +%s) - start)) 秒"
