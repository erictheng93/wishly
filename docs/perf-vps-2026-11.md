# 低規 VPS 效能評估與優化（2026-11）

目標：在 **1 vCPU / 1 GB RAM、API 與 Postgres 同機** 的主機上，求 5 分鐘內能穩定承受的最大同時在線使用者數，
條件 p95 < 500 ms、p99 < 1 s、錯誤率 < 1%。壓測工具與重現步驟見 `scripts/loadtest/README.md`。

## 結果

每輪都從全新資料庫開始（種子資料後立即壓測），1 分鐘爬升、維持 5 分鐘。

| 版本 | 人數 | p95 | p99 | 錯誤率 | 判定 |
|---|---|---|---|---|---|
| 原始程式（TCP、連線池 10） | 650 | 348 ms | 658 ms | 0% | 通過 |
| 原始程式 | 800 | 1,326 ms | 1,768 ms | 0% | 不合格 |
| 優化後（Unix socket、連線池 5） | **3,450** | 329 ms | 600 ms | 0% | 通過 |
| 優化後 | 3,600 | 516 ms | 893 ms | 0% | 不合格（p95） |

原始程式約 650–800 人，優化後約 3,450–3,600 人，約 **5.3 倍**。優化後每位使用者同時開著一條 SSE（3,450 條全數維持，0 失敗）。

> 更正：最初版本寫「400 → 3,450（8.6 倍）」。當時的會員劇本讀錯 `GET /wishlists` 的回應欄位（`data`），
> 「我的清單 / 進度」兩個請求從未執行；原始程式的 400 人也是在累積了數十萬筆壓測資料的資料庫上量的。
> 上表為修正劇本後、兩邊都從全新資料庫重新量測的結果。

量的是**來源站最壞情況**：每個請求都打到 VPS。正式環境的公開清單與探索頁有 Cloudflare Cache Rule（`s-maxage=10`），
訪客請求（約占 60% 使用者）大多在邊緣就回應，實際容量會更高。

### 劇本

每位虛擬使用者每 3–8 秒換一頁（約 0.6 req/s/人）。訪客 60%：分享頁（SSR 一次 + 瀏覽器帶 ETag 一次）、`/public/status`、`/me`（401）、
回報瀏覽、一半機率看探索頁。會員 40%：儀表板五個請求、自己的清單與進度、好友的 friends 清單，約 1/5 機率捐助後取消。
資料：3,000 位使用者、6,000 份清單、60,000 個品項。模擬主機以 cgroup 限制 API 與 DB 容器共用 1 顆 CPU、1 GB。

### 逐步改善（優化過程中的爬升測試崩潰點，非 5 分鐘驗收；使用修正前的劇本，僅供相對比較）

| 改動 | p95 開始失控的人數 |
|---|---|
| 原始 | ~400（舊劇本、資料庫已累積大量資料） |
| slug 走索引 + 品項索引 | ~1,200 |
| 瀏覽紀錄非同步 commit、探索頁快取 | ~2,000 |
| `/me` 合併查詢、唯讀旗標快取、`config::get` 免複製等 | ~3,100 |
| mimalloc + LTO、Unix socket、連線池 5（含 SSE，90 秒維持） | ~3,700 |

## 發現與修正（依影響排序）

1. **分享頁整表掃描（最大問題）**。`wishlists.slug` 是 `char(10)`，sqlx 以 `text` 綁參數，`slug = $1` 被轉成 `slug::text = $1`，
   唯一索引用不上。分享頁主查詢平均 45 ms，占 DB 時間 95%。改為 `$1::bpchar`（公開頁、SSE、檢舉、支持品項、組織 / 事件）。
2. **ETag 的 `max(updated_at)` 整表掃描 `wishlist_items`**：刻意含已刪除品項，所以用不上 `deleted_at IS NULL` 的部分索引。
   0064 新增 `(wishlist_id, updated_at)` 完整索引。
3. **每次瀏覽 3 次 fsync**：`POST /views` 原本是兩次限流 upsert + 一次 insert，各自 commit。1 vCPU 主機上後端大多在等 `WALWrite` / `WALSync`。
   改為限流在程序內計數（`ratelimit::allow_local`）、寫入單一敘述並以 `set_config('synchronous_commit','off',true)` 非同步 commit。
4. **每個 SQL 往返的固定成本才是主要開銷**。實測 Postgres 行程 CPU 有一半以上不是查詢執行，而是協定、快照與行程切換；
   API 端也有約 30% 在 socket 系統呼叫。因此：`GET /me` 5 次查詢合為 1 次；分享頁的 `has_support` 併入主查詢；
   探索頁（每位訪客都一樣）程序內快取 10 秒；`read_only` 旗標（每個寫入與 `/public/status` 都讀）快取 1 秒，admin 改旗標立即失效。
   兩者由 `MICRO_CACHE` 控制，測試用的 `Config::dev()` 為關。
5. **`config::get()` 每次複製整份設定**（含多個字串與 S3 / 綠界設定，每個請求呼叫多次，品項圖片網址每項一次）：改回傳 `&'static Config`。
6. **glibc malloc 占 API 約 15% CPU**：改 mimalloc，API 常駐記憶體由約 150 MB 降到 30–70 MB；release 開 LTO、`codegen-units = 1`。
7. **公開 SSE 上限 2,000 先於 CPU 成為人數上限**：實測每條約 10 KB，放寬到 5,000。
8. 連線池可由 `DB_POOL_MAX` 調整：1 vCPU 上 5 條比 10 條延遲更低（同 3,300 人 90 秒：p95 269 ms 對 421 ms）。

所有修改都通過完整後端測試（109 個 suite）、`cargo fmt`、`clippy -D warnings`；新增 `tests/micro_cache.rs` 驗證快取的過期與失效。

## 取捨

- 瀏覽紀錄非同步 commit：主機當機（不是程式崩潰）時最近約 0.6 秒的瀏覽紀錄可能遺失；一致性不受影響。捐助、金流等業務寫入完全沒動。
- 瀏覽限流改在程序內：重啟歸零、多實例時各算各的（換版瞬間最多兩倍）。它只擋灌水，不是安全邊界；IP 也因此不再寫進 `rate_limits`。
- 探索頁最多晚 10 秒（與 CDN 原本的 `s-maxage=10` 相同）；`read_only` 在其他實例最多晚 1 秒生效。

## 部署建議（不需改程式，Dokploy 設定）

1. **API 經 Unix socket 連 DB**：在 Dokploy 把 `db` 的 `/var/run/postgresql` 掛成具名 volume，同一個 volume 掛進 `api`，
   `DATABASE_URL=postgres://wishly:<密碼>@localhost/wishly?host=/var/run/postgresql`。實測同負載吞吐 +13%、p95 減半
   （TCP 經 Docker bridge 的 conntrack / veth 開銷在 1 vCPU 上很明顯）。
2. `DB_POOL_MAX=5`。
3. Postgres（1 GB 主機）：`shared_buffers=256MB`、`effective_cache_size=512MB`、`work_mem=4MB`、`max_connections=30`、`jit=off`。
4. cloudflared 直接指向 API 容器，不要再經 Dokploy 的 Traefik（每個請求少一層代理的 CPU）。
5. 維持 Cloudflare 對 `/api/v1/public/*` 的 Cache Rule（已於 2026-10-17 建立）；這是訪客流量最大的槓桿。

## 尚未做、可再提升的

- **session 驗證快取**：每個登入請求都查一次 `sessions`，約占全部 SQL 敘述的 23%。程序內快取 1–2 秒可大幅減少，但牽涉登出、
  撤銷裝置、停權與 MFA 狀態的即時性，需另外設計失效機制，這次刻意不做。
- 分享頁的品項查詢併入主查詢（每次完整載入少一次往返）、站內通知列表與未讀數合併、好友 / 封鎖判斷合併。
- 分享頁 JSON 改用 `#[derive(Serialize)]` 結構直接序列化，取代 `json!` 建樹（API 端 CPU 的一部分）。
- 前端：會員每換一頁就重抓 `/me`，可沿用已載入的使用者狀態。
