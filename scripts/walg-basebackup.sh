#!/usr/bin/env bash
# 方案 A：每日 WAL-G 基底備份並清理舊備份（保留 8 份 ≈ PITR 7 天，舊 WAL 一併刪除）。
# VPS 上由 cron 每日執行，見 docs/deploy-runbook.md §6。
set -euo pipefail
DB=${DB_CONTAINER:-wishly-db-2y7t3r-db-1}
docker exec -u postgres "$DB" /walg/archive.sh backup-push /var/lib/postgresql/data
docker exec -u postgres "$DB" /walg/archive.sh delete retain FULL 8 --confirm
docker exec -u postgres "$DB" /walg/archive.sh backup-list | tail -n +2 | wc -l | xargs echo "ok base backups:"
