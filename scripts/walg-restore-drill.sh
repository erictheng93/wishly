#!/usr/bin/env bash
# PITR 還原演練：從 R2 取最新基底備份 + 重播歸檔 WAL 到暫存容器 → 對帳 → 印出耗時，結束後移除容器。
# 用法：scripts/walg-restore-drill.sh [recovery_target_time，例如 '2026-10-27 09:00:00+00'；省略則重播到最後]
set -euo pipefail
TARGET=${1:-}
SQL=${SQL:-$(dirname "$0")/db-reconcile.sql}
IMAGE=${PG_IMAGE:-postgis/postgis:16-3.4}
WALG=${WALG_DIR:-/opt/wishly-backup/walg}
C=wishly-pitr-drill-$$
start=$(date +%s)
trap 'docker rm -f -v "$C" >/dev/null 2>&1 || true' EXIT

# archive_mode=off：還原出的實例繼承正式設定，不關掉會把新時間線的 WAL 推進正式歸檔
docker run -d --name "$C" --network bridge --user 999 -v "$WALG:/walg:ro" -e TARGET="$TARGET" \
    --entrypoint sh "$IMAGE" -c '
set -e; D=/var/lib/postgresql/data
/walg/archive.sh backup-fetch "$D" LATEST
chmod 700 "$D"; touch "$D/recovery.signal"
{ echo "archive_mode = off"
  echo "restore_command = '\''/walg/archive.sh wal-fetch %f %p'\''"
  echo "recovery_target_action = promote"
  [ -n "$TARGET" ] && echo "recovery_target_time = '\''$TARGET'\''"
} >> "$D/postgresql.auto.conf"
exec postgres' >/dev/null

# 重播完並升格後 pg_is_in_recovery() 才會是 f
for _ in $(seq 600); do
    [ "$(docker exec "$C" psql -U wishly -d wishly -Atc 'select pg_is_in_recovery()' 2>/dev/null)" = f ] && break
    docker inspect -f '{{.State.Running}}' "$C" | grep -q true || { docker logs "$C" | tail -20; exit 1; }
    sleep 1
done
[ "$(docker exec "$C" psql -U wishly -d wishly -Atc 'select pg_is_in_recovery()')" = f ]
echo "重播 WAL 段數：$(docker logs "$C" 2>&1 | grep -c 'restored log file')"
docker logs "$C" 2>&1 | grep -E 'redo done|recovery stopping|last completed transaction' || true
docker exec -i "$C" psql -U wishly -d wishly -X <"$SQL"
echo "PITR 演練通過，耗時 $(($(date +%s) - start)) 秒"
