#!/usr/bin/env bash
# 快速比較用：30 秒爬升到 N、維持 HOLD（預設 90s），含 SSE。印出 hold 階段的 p95 / p99 / 錯誤率與 RPS。
set -euo pipefail
N=$1; TAG=${2:-q}; OUT=${OUT:-/tmp/wishly-lt}; mkdir -p "$OUT"
K6=${K6:-k6}; USERS=${USERS:-3000}; HOLD=${HOLD:-90s}
here=$(dirname "$0")
# 每輪重置限流計數：同一批種子帳號連續壓測會累積每小時的捐助額度，造成假的 429
docker exec vpsdb psql -U wishly -d wishly -qc "DELETE FROM rate_limits" >/dev/null 2>&1 || true
V=$(( N * 60 / 100 )); M=$(( N - V ))
node "$here/sse-hold.mjs" http://127.0.0.1:8080 "$V" "$USERS" 30 "$M" > "$OUT/sse-$TAG.log" 2>&1 &
sse=$!; trap 'kill $sse 2>/dev/null || true' EXIT
taskset -c 1-3 "$K6" run -q -e USERS="$USERS" -e PEAK="$N" -e RAMP=30s -e HOLD="$HOLD" --out csv="$OUT/$TAG.csv.gz" "$here/users.js" > "$OUT/$TAG.txt" 2>&1 || true
python3 "$here/analyze.py" "$OUT/$TAG.csv.gz" | grep HOLD
awk -F, '$1=="http_reqs" && $0 ~ /phase=hold/ {n++} END {print "hold reqs:", n}' "$OUT/$TAG.csv.gz"
echo "SSE: $(tail -1 "$OUT/sse-$TAG.log")"
