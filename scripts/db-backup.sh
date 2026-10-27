#!/usr/bin/env bash
# 排程邏輯備份（checklist §2.1 方案 B）：pg_dump（zstd 壓縮）→ 驗證可讀 → 本機保留 → 上傳 R2。
# VPS 上由 cron 每小時執行，見 docs/deploy-runbook.md §6。
set -euo pipefail
umask 077 # dump 含 email、密碼雜湊，只給 root 讀
DB=${DB_CONTAINER:-wishly-db-2y7t3r-db-1}
DIR=${BACKUP_DIR:-/var/backups/wishly}
ENV_FILE=${R2_ENV_FILE:-/etc/wishly-backup/r2.env}
KEEP_LOCAL=${KEEP_LOCAL:-48}
RCLONE_IMAGE=rclone/rclone:1.68.2

mkdir -p "$DIR"
f="$DIR/wishly-$(date -u +%Y%m%dT%H%M%SZ).dump"
# -Fc 自帶壓縮，壓縮後才落地與上傳；pg_restore 直接讀，不需另外解壓
docker exec "$DB" pg_dump -U wishly -d wishly -Fc -Z zstd >"$f.part"
# 目錄可列出才算完整，避免把截斷的檔案當成備份
docker exec -i "$DB" pg_restore -l >/dev/null <"$f.part"
mv "$f.part" "$f"

if [ -f "$ENV_FILE" ]; then
    # shellcheck disable=SC1090
    . "$ENV_FILE" # R2_BUCKET、RCLONE_CONFIG_R2_*、可選 PING_URL
    # --bind 0.0.0.0 強制 IPv4：token 只放行 VPS 的 IPv4，VPS 的 IPv6 是動態位址
    docker run --rm --network host --env-file "$ENV_FILE" -v "$DIR:/data:ro" "$RCLONE_IMAGE" \
        copyto "/data/$(basename "$f")" "r2:$R2_BUCKET/postgres/$(basename "$f")" --s3-no-check-bucket --bind 0.0.0.0
fi

# 方案 A 的 WAL 歸檔：最近一次失敗晚於成功就中止，不 ping，讓告警響（歸檔卡住時 WAL 會堆滿磁碟）
if [ "$(docker exec "$DB" psql -U wishly -d wishly -Atc "select coalesce(last_failed_time > coalesce(last_archived_time, '-infinity'), false) from pg_stat_archiver")" = t ]; then
    echo "WAL 歸檔失敗，見 pg_stat_archiver" >&2
    exit 1
fi
# 方案 A 的基底備份每日 03:47 UTC 一次；最新一份超過時限（預設 26 小時，留 2 小時餘裕）就中止
latest=$(docker exec -u postgres "$DB" /walg/archive.sh backup-list 2>/dev/null | tail -n +2 | awk '{print $2}' | sort | tail -1)
if [ -z "$latest" ] || [ $(($(date +%s) - $(date -d "$latest" +%s))) -gt $((${BASE_MAX_AGE_HOURS:-26} * 3600)) ]; then
    echo "WAL-G 基底備份過舊或不存在（最新：${latest:-無}），見 journalctl -t wishly-walg" >&2
    exit 1
fi
ls -1t "$DIR"/wishly-*.dump | tail -n +$((KEEP_LOCAL + 1)) | xargs -r rm --
# 全部成功才回報；沒收到 ping 由監控服務告警（備份靜默失敗最危險）
if [ -n "${PING_URL:-}" ]; then curl -fsS -m 10 "$PING_URL" >/dev/null; fi
echo "ok $f $(wc -c <"$f") bytes"
