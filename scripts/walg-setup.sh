#!/usr/bin/env bash
# 在 VPS 上安裝 WAL-G 並由 /etc/wishly-backup/r2.env 產生 walg.env（可重複執行）。
# db 容器以 /opt/wishly-backup/walg:/walg:ro 掛載此目錄，見 docs/deploy-runbook.md §6、§8。
set -euo pipefail
VER=v3.0.9
W=${WALG_DIR:-/opt/wishly-backup/walg}
ENV_FILE=${R2_ENV_FILE:-/etc/wishly-backup/r2.env}
# 20.04 版連結 glibc 2.31，postgis/postgis:16-3.4（Debian 11）才跑得動
U=https://github.com/wal-g/wal-g/releases/download/$VER/wal-g-pg-20.04-amd64

mkdir -p "$W"
cd "$W"
if ! ./wal-g --version 2>/dev/null | grep -q "$VER"; then
    curl -fsSL -o wal-g.tar.gz "$U.tar.gz"
    curl -fsSL -o wal-g.sha256 "$U.tar.gz.sha256"
    echo "$(cut -d' ' -f1 wal-g.sha256)  wal-g.tar.gz" | sha256sum -c -
    tar xzf wal-g.tar.gz
    mv wal-g-pg-20.04-amd64 wal-g
    rm wal-g.tar.gz wal-g.sha256
    chmod 755 wal-g
fi

# shellcheck disable=SC1090
. "$ENV_FILE"
(
    umask 077
    cat >walg.env <<EOF
WALG_S3_PREFIX=s3://$R2_BUCKET/wal-g
AWS_ACCESS_KEY_ID=$RCLONE_CONFIG_R2_ACCESS_KEY_ID
AWS_SECRET_ACCESS_KEY=$RCLONE_CONFIG_R2_SECRET_ACCESS_KEY
AWS_ENDPOINT=$RCLONE_CONFIG_R2_ENDPOINT
AWS_REGION=auto
AWS_S3_FORCE_PATH_STYLE=true
WALG_COMPRESSION_METHOD=zstd
PGHOST=/var/run/postgresql
PGUSER=wishly
PGDATABASE=wishly
EOF
)
cat >archive.sh <<'EOF'
#!/bin/sh
# archive_command / restore_command 的入口：載入 R2 設定後呼叫 wal-g（%p、%f 由 PostgreSQL 傳入）
set -a; . /walg/walg.env; set +a
exec /walg/wal-g "$@"
EOF
chmod 755 archive.sh
# 容器內 postgres 的 uid 是 999
chown 999:999 walg.env
chmod 400 walg.env
echo "ok $("$W/wal-g" --version | head -1)"
