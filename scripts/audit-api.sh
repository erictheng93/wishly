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
# 同理：未啟用的 SQLite driver 讓鎖檔含 libsqlite3-sys（RUSTSEC-2026-0459，需 >=0.39，sqlx 0.9 尚不允許）。
if ! printf '%s\n' "$deps" | grep -q '^libsqlite3-sys v'; then
    ignores="$ignores --ignore RUSTSEC-2026-0459"
fi
# shellcheck disable=SC2086
cargo audit $ignores
