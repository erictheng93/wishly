#!/usr/bin/env sh
set -eu
cd "$(dirname "$0")/../apps/api"
# sqlx 的鎖檔包含未啟用 MySQL driver 的 rsa。RUSTSEC-2023-0071 尚無修補；
# 僅在所有平台的實際依賴樹沒有 rsa 時略過，新增啟用路徑會恢復完整稽核。
deps=$(cargo tree --locked --target all --prefix none)
ignores=""
if ! printf '%s\n' "$deps" | grep -q '^rsa v'; then
    ignores="--ignore RUSTSEC-2023-0071"
fi
# sqlx 0.8 依賴 webpki-roots 0.26.11：它只是轉出 1.x 的憑證、不會再出修補版。
# RUSTSEC-2026-0381 只在樹上所有 webpki-roots 1.x 都 >= 1.0.10（已修補）時略過；升級 sqlx 0.9 後可移除。
if ! printf '%s\n' "$deps" | grep -Eq '^webpki-roots v1\.0\.[0-9]( |$)'; then
    ignores="$ignores --ignore RUSTSEC-2026-0381"
fi
# shellcheck disable=SC2086
cargo audit $ignores
