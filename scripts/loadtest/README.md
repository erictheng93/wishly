# 低規 VPS 壓測（虛擬使用者）

模擬「1 vCPU / 1 GB RAM、API 與 Postgres 同機」的主機，用 k6 跑真實瀏覽行為，量測在 p95 < 500ms、p99 < 1s、錯誤率 < 1% 下
**5 分鐘內能穩定承受的最大同時在線使用者數**。結果與分析見 `docs/perf-vps-2026-11.md`。

**只在本機 / 壓測環境使用**：`seed.sql` 會寫入大量假帳號與可登入的 session。

## 劇本（`users.js`）

每個 VU = 一位同時在線的使用者，換頁之間思考 3–8 秒（約 0.6 req/s/人）。

- 訪客 60%：開分享頁（SSR 抓一次 + 瀏覽器帶 ETag 再抓一次）、`/public/status`、回報瀏覽（匿名訪客已不再打 `/me`）；一半機率逛探索頁。
- 會員 40%：儀表板（`/me`、導覽、好友申請數、站內通知、我的清單）→ 自己的清單與進度 → 好友的 friends 清單 → 約 1/5 機率捐助一個品項後取消。
- 另由 `sse-hold.mjs` 讓**每位**使用者開一條 SSE（訪客：分享頁即時更新；會員：站內通知串流）。

## 步驟

```sh
# 1) 模擬主機：cgroup v1 /vps = 1 顆 CPU（固定 CPU 0）+ 1 GB，DB 容器與 API 都放進去；k6 跑在 CPU 1-3
for c in cpu cpuacct cpuset memory; do mkdir -p /sys/fs/cgroup/$c/vps; done
echo 100000 > /sys/fs/cgroup/cpu/vps/cpu.cfs_quota_us
echo 0 > /sys/fs/cgroup/cpuset/vps/cpuset.cpus; cat /sys/fs/cgroup/cpuset/cpuset.mems > /sys/fs/cgroup/cpuset/vps/cpuset.mems
echo $((1<<30)) > /sys/fs/cgroup/memory/vps/memory.limit_in_bytes
mkdir -p /tmp/wishly-pgsock && chmod 777 /tmp/wishly-pgsock
docker run -d --name vpsdb --cgroup-parent=/vps -e POSTGRES_USER=wishly -e POSTGRES_PASSWORD=wishly -e POSTGRES_DB=wishly \
  -v /tmp/wishly-pgsock:/var/run/postgresql -p 5432:5432 postgis/postgis:16-3.6

# 2) API（release；在 cgroup 內啟動，預設經 Unix socket 連 DB、連線池 5；改用 TCP：DATABASE_URL=postgres://…@<容器 IP>:5432/wishly）
cargo build --release --manifest-path apps/api/Cargo.toml
scripts/loadtest/run-api.sh

# 3) 種子資料：3000 位使用者、6000 份清單、60000 個品項
psql postgres://wishly:wishly@localhost:5432/wishly -v users=3000 -f scripts/loadtest/seed.sql

# 4) 驗收：1 分鐘爬升到 N 人、維持 5 分鐘
K6=/path/to/k6 scripts/loadtest/accept.sh 3500
# 快速比較（30 秒爬升、維持 90 秒）
K6=/path/to/k6 scripts/loadtest/quick.sh 3300 mytag
```

`analyze.py` 會印出每 10 秒的 VU / RPS / p95 / p99 / 錯誤率，與 hold 階段各端點的延遲。

## 注意

- 量的是**來源站最壞情況**：正式環境的公開清單與探索頁有 Cloudflare 快取（`s-maxage=10`），大部分訪客請求不會到 VPS。
- API 必須以 `TRUSTED_PROXY=cloudflare` 啟動（`run-api.sh` 已設），每個 VU 才會以 `CF-Connecting-IP` 被視為不同 IP。
- 每輪開始會清空 `rate_limits`：同一批種子帳號連續壓測會累積每小時的捐助額度，產生非真實的 429。
- 不要在 cgroup 外啟動 API 再搬進去：tokio 啟動時依可見 CPU 數決定 worker 數，會與真正的 1 vCPU 主機不同。
