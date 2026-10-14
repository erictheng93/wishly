#!/usr/bin/env sh
set -eu
cd "$(dirname "$0")/../apps/api"
# sqlx 的鎖檔包含未啟用 MySQL driver 的 rsa。RUSTSEC-2023-0071 尚無修補；
# 僅在所有平台的實際依賴樹沒有 rsa 時略過，新增啟用路徑會恢復完整稽核。
deps=$(cargo tree --locked --target all --prefix none)
if printf '%s\n' "$deps" | grep -q '^rsa v'; then
    cargo audit
else
    cargo audit --ignore RUSTSEC-2023-0071
fi
