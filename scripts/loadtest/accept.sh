#!/usr/bin/env bash
# 驗收：1 分鐘爬升到 N 位同時在線使用者、維持 5 分鐘；每位使用者同時開著一條 SSE（訪客 60% 分享頁、會員 40% 站內通知）。
#   scripts/loadtest/accept.sh <N> [輸出目錄]
# 判定（只看 hold 階段）：p95 < 500ms、p99 < 1s、錯誤率 < 1%（k6 thresholds），另印 SSE 失敗數。
set -euo pipefail
N=$1; OUT=${2:-/tmp/wishly-lt}; mkdir -p "$OUT"
K6=${K6:-k6}; USERS=${USERS:-3000}; HOLD=${HOLD:-5m}
here=$(dirname "$0")
# 每輪重置限流計數：同一批種子帳號連續壓測會累積每小時的捐助額度，造成假的 429
docker exec vpsdb psql -U wishly -d wishly -qc "DELETE FROM rate_limits" >/dev/null 2>&1 || true
V=$(( N * 60 / 100 )); M=$(( N - V ))
node "$here/sse-hold.mjs" http://127.0.0.1:8080 "$V" "$USERS" 60 "$M" > "$OUT/sse-$N.log" 2>&1 &
sse=$!
trap 'kill $sse 2>/dev/null || true' EXIT
taskset -c 1-3 "$K6" run -q -e USERS="$USERS" -e PEAK="$N" -e RAMP=60s -e HOLD="$HOLD" \
  --out csv="$OUT/accept-$N.csv.gz" "$here/users.js" > "$OUT/accept-$N.txt" 2>&1 || true
python3 "$here/analyze.py" "$OUT/accept-$N.csv.gz" --by-endpoint | sed -n '/^HOLD/,$p'
grep -E "✓|✗" "$OUT/accept-$N.txt" | grep -E "phase:hold" || true
echo "SSE: $(tail -1 "$OUT/sse-$N.log")"
