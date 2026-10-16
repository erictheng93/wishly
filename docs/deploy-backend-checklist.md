# 後端部署檢查清單（apps/api → Linode VPS + Dokploy + Cloudflare Tunnel）

**部署架構（PRD D49，2026-10-15 定案）**：前端 Cloudflare Pages；後端與 PostgreSQL 同機部署在 Linode（Akamai）VPS 東京節點，以 Dokploy 管理容器，經 Cloudflare Tunnel 對外；圖片用 Cloudflare R2。單機單實例。

```
使用者 → Cloudflare（WAF／CDN）→ Tunnel（cloudflared，出站連線）→ Traefik（Dokploy）→ wishly-api :8080 → PostgreSQL（同機）
前端：Cloudflare Pages（app.<域名>）        圖片：Cloudflare R2（S3_PUBLIC_BASE，自訂網域）
```

本文件只列「上線前必須成立的條件」，每項附驗證方式。勾選請附日期與證據（指令輸出、截圖、commit），否則視為未完成。

> 依據：`apps/api/src/config.rs`、`apps/api/.env.example`、README〈環境變數〉〈部署〉、PRD D49、`docs/02` 部署與備份列。
> 標示「**需實測**」的項目是關於 Dokploy／Traefik／Tunnel 的行為假設，本文件撰寫時未在實機驗證，不得當成事實。

---

## 0. Repo 內缺口現況

| # | 原缺口 | 現況 |
|---|---|---|
| 0.1 | 沒有 Dockerfile | ✅ 已解決：`apps/api/Dockerfile`、`.dockerignore`，CI `image` job 驗證可建置（build context 為 repo 根目錄） |
| 0.2 | 沒有健康檢查端點 | ✅ 已解決：`GET /healthz`（不碰 DB）、`GET /readyz`（`SELECT 1`，2 秒逾時）；掛在根路徑 |
| 0.3 | 背景工作在 Cloud Run 會被凍結 | ✅ **因部署決策而不成立**：VPS 行程長駐，`spawn_worker`／`spawn_cleanup` 照常運作，不實作 `/internal/jobs/tick`。仍有小風險見 §0.6 |
| 0.4 | 多實例是否冪等 | ⏸ 單實例時不成立；**擴充為多實例前必須先確認**（`expire_due`、`close_due`、`remind_closing`、`tick_funding`、寄信領取）。在此之前 Dokploy 的 replicas 必須維持 1 |
| 0.5 | 網域未定 | ⛔ 仍待定（`docs/domain-names.md`）；前端與 API 須同一 registrable domain |
| 0.6 | 背景任務 panic 會靜默終止 | ⚠ `tokio::spawn` 的迴圈若 panic，該任務就此停止而行程仍存活（`/healthz` 仍 200）。需決定：任務外層加 panic 捕捉並重啟，或讓行程在背景任務結束時退出由 Dokploy 重啟；並加「逾期認領未釋放」告警（§6） |

---

## 1. 前置決策

- [ ] 正式網域與子網域：`app.<域名>`（Pages）、`api.<域名>`（Tunnel）。`ws_session` 為 `SameSite=Lax` 且不帶 `Domain`，前端與 API 必須同一 registrable domain。
- [ ] **RPO／RTO 取捨（見 §2.1）**：PRD 目標 RPO ≤ 5 分鐘、RTO ≤ 1 小時。RPO ≤ 5 分鐘需要 WAL 歸檔；只用排程備份則 RPO 等於備份間隔。由產品負責人明確選擇並寫入 PRD。
- [ ] 環境：staging 與 production 是否同一台 VPS（以不同 Dokploy 專案與資料庫隔離）還是兩台？同機的 staging 會與正式搶資源。
- [ ] VPS 規格（CPU／RAM／磁碟）與單機容量上限的預期（SSE 連線數、尖峰認領 QPS）。

## 2. 基礎設施

### 2.1 PostgreSQL（同機自管）— 最大風險區
- [ ] 版本 16；映像選擇：目前 migration **不需要** PostGIS（`0001` 註明 relief 才建，`uuid_v7()` 由 migration 自行建立，無 `CREATE EXTENSION`）。CI 與本機用 `postgis/postgis:16-3.4`，建議正式也用同一映像以保持一致，P2-D 時不必換庫。以空庫完整跑 migration 驗證。
- [ ] 資料目錄在**具名 volume／獨立磁碟**，不放在容器可寫層；磁碟空間告警（WAL、日誌、備份暫存）。
- [ ] 不對外開放：只監聽容器網路／回送，`5432` 不暴露在公網；密碼獨立且 ≥ 32 字元。
- [ ] 連線：後端 `max_connections(10)`；Postgres `max_connections` 大於此值並預留管理連線。單實例不需要 PgBouncer。
- [ ] **備份（二選一，寫入決策）**
  - A. WAL 歸檔 + 基底備份（pgBackRest 或 WAL-G，目的地為 R2）：可達 RPO ≈ 數分鐘、支援 PITR。
  - B. 排程 `pg_dump`（Dokploy 內建備份是否為此、能否做 PITR：**需實測**）：RPO = 備份間隔；若選此項，PRD 的 RPO ≤ 5 分鐘必須改寫並由產品負責人接受。
- [ ] 備份目的地在**不同供應商／帳號**（R2），且存取金鑰與 VPS 上的應用金鑰分開。
- [ ] **做過一次還原演練**：還原到暫存實例，啟動後端對帳查詢（`assert_all_reconciled` 同類檢查）通過；記錄實際 RTO。之後每季一次。
- [ ] Linode 整機備份／快照視為輔助，不等於資料庫一致性備份。
- [ ] 單點故障認知：整機掛掉全站掛掉；寫下恢復步驟（新 VPS → Dokploy → 還原 DB → 設環境變數 → Tunnel）並計時。

### 2.2 物件儲存（Cloudflare R2，已決）
程式以手寫 SigV4 對接 S3 相容儲存（`uploads.rs`，**path-style**），所以 R2 不需改程式。流程：前端 presigned PUT 到 `incoming/{user_id}/…` → 後端 confirm 時去 EXIF／GPS 重新編碼，寫入公開 key（`covers/…`、`items/…`）→ 刪除 `incoming/`。

- [ ] bucket 建立；**建立時選擇 location hint（APAC）**——之後無法變更（**需於 Cloudflare 後台確認**）。staging／production 各自獨立 bucket。
- [ ] 環境變數：`S3_ENDPOINT=https://<account_id>.r2.cloudflarestorage.com`、`S3_REGION=auto`、`S3_BUCKET`、`S3_PUBLIC_BASE=https://<media 自訂網域>`。
- [ ] API Token：只限該 bucket 的 Object Read & Write，不給帳號層級權限；與其他用途的 token 分開。程式啟動後會嘗試 `PUT /{bucket}`（`ensure_bucket`，結果被忽略）：在受限 token 下會被拒絕，屬預期，不影響運作。
- [ ] **公開讀取用自訂網域綁定 bucket**（例如 `media.<域名>`），不要使用 `r2.dev` 公開網址（有速率限制、不適合正式環境）。
- [ ] ⚠ **`incoming/` 不得對外可讀**：bucket 一旦公開，所有前綴都可讀，而 `incoming/` 放的是**尚未去除 EXIF／GPS** 的原圖，且確認前會停留。兩道防線都要做：
  1. Cloudflare WAF 規則：對 `media.<域名>` 的 `URI Path starts with /incoming/` 一律封鎖。
  2. R2 物件生命週期規則：前綴 `incoming/` 於 1 天後刪除（兜住上傳後未 confirm 的殘留）。
  - 驗證：用瀏覽器直接開一個 `incoming/…` 物件網址得到 403／404。
- [ ] CORS：允許前端網域對 bucket 的 `PUT`，允許標頭 `Content-Type`（presigned PUT 簽入 content-type 與 content-length）。
- [ ] **需實測**：R2 對「簽入 `content-length` 的 presigned PUT」的行為（`presign()` 會把 `content-length` 與 `content-type` 簽進 SignedHeaders）；以真實 R2 與瀏覽器上傳一張圖驗證，包含超過 `Content-Length` 宣告大小時被拒絕。
- [ ] 物件版本與備份：PRD 要求 R2 物件版本保留 30 天——R2 的版本控制／保護機制與設定方式需於後台確認後寫入決策。
- [ ] 驗證：上傳一張圖，經 `S3_PUBLIC_BASE` 在瀏覽器開得到；確認重新編碼後的檔案不含 EXIF；頭像轉存（issue #6）能寫入。

### 2.3 VPS 主機
- [ ] Linode 東京節點；OS 為受支援 LTS；`unattended-upgrades`（或等效）啟用安全更新。
- [ ] 防火牆（Linode Cloud Firewall 與主機 ufw 擇一以上）：**入站全部拒絕**，包含 80／443（Tunnel 為出站連線）；SSH 限來源 IP，或改經 Tailscale／Cloudflare Access；停用密碼登入、只用金鑰；fail2ban 或等效。
- [ ] **Dokploy 管理介面不得公開在公網**：預設埠（3000）只允許你的來源 IP／VPN／Tunnel 加 Access 保護。**需實測**目前安裝是否已綁在公網。
- [ ] Docker：日誌輪替（`max-size`／`max-file`），避免日誌塞滿磁碟。
- [ ] 時間同步（chrony／systemd-timesyncd）；時區 UTC。
- [ ] 資源上限：容器記憶體／CPU 限制，避免單一服務（含 `image` 解碼）拖垮同機的 Postgres。

### 2.4 Dokploy
- [ ] 服務類型：Application（Dockerfile 路徑 `apps/api/Dockerfile`、**build context 為 repo 根目錄**）或使用預先 build 的映像；記錄採用哪種。
- [ ] **replicas = 1**（見 §0.4）。
- [ ] 環境變數放在 Dokploy（§3），不在 repo；確認匯出／備份 Dokploy 設定的方式（遺失 Dokploy 設定 = 遺失全部機密）。
- [ ] 重啟策略：失敗自動重啟；理解「migration 失敗 → 行程退出 → 反覆重啟」是預期行為，值班手冊寫明並設告警。
- [ ] 換版行為：程式**不處理 SIGTERM**（已移除優雅關閉，2026-10-15），收到終止訊號即結束，進行中的請求會被切斷；認領有 `Idempotency-Key`，客戶端重試可恢復。**需實測**：Dokploy 部署新版時是否先啟新容器再停舊容器（短暫兩個實例並存，會影響背景工作與 migration 順序）。
- [ ] 回滾：保留前一個部署版本，確認一鍵回滾可用且 migration 僅向前相容（§8）。
- [ ] `RUST_LOG=info`（image 已預設）；日誌可在 Dokploy 檢視並有保存期限。

### 2.5 Cloudflare Tunnel
- [ ] cloudflared 以容器或系統服務執行並設為自動重啟；憑證／token 當成機密保管。
- [ ] 公開主機名稱 `api.<域名>` → 指向 Traefik 或直接指向後端容器 `:8080`（擇一，記錄選擇）。**需實測**：該路徑是否保留 `CF-Connecting-IP`、對 SSE 是否不緩衝。
- [ ] 後端 `BIND`：若 Tunnel 與後端在同一 Docker 網路，需 `0.0.0.0:8080`（容器內）；此時 `bind_warning` 會印 WARN，屬預期，**前提是主機無任何入站規則可繞過 Tunnel**（§2.3），並記錄這個放行理由。
- [ ] 驗證 API 不可被繞過：從外部直接連 VPS 公網 IP 的 80／443／8080 全部連不上。
- [ ] Tunnel 斷線的行為：cloudflared 重連時 SSE 會斷，client 依 `retry: 5s` 重連；Tunnel 掛掉 = API 對外不可用，要有告警。

## 3. 環境變數與機密（production 缺漏即拒絕啟動）

`APP_ENV` 未設視為 production（fail closed）。以下全數必填，缺漏會 panic 並指出變數名。

| 變數 | 要求 | 備註 |
|---|---|---|
| `APP_ENV` | `production` | 明確設定 |
| `DATABASE_URL` | 必填 | 指向同機／同 Docker 網路的 Postgres |
| `BIND` | `0.0.0.0:8080` | 容器內；見 §2.5 |
| `APP_URL` | 前端正式網址 | CORS 只允許此來源（並帶憑證） |
| `API_BASE_URL` | API 正式網址 | |
| `OTP_PEPPER`、`OAUTH_SECRET`、`UNSUB_SECRET` | 各自獨立、≥32 字元（`openssl rand -hex 32`） | 更換 `UNSUB_SECRET` 會使既有退訂連結與清單存取權杖失效 |
| `CF_ACCOUNT_ID`、`CF_EMAIL_API_TOKEN`、`MAIL_FROM` | 必填 | `MAIL_FROM` 網域須已在 Cloudflare Email Service 驗證 |
| `TRUSTED_PROXY` | `cloudflare` | 前提：API 只能經 Tunnel 到達（§2.3、§2.5） |
| `CF_ZONE_ID`、`CF_CACHE_PURGE_TOKEN` | `TRUSTED_PROXY=cloudflare` 時必填 | token 只給 Zone → Cache Purge |
| `TURNSTILE_SECRET` | 必填 | 與前端 `NUXT_PUBLIC_TURNSTILE_SITE_KEY` 成對 |
| `SHIPPING_ENC_KEY` | base64 的 32 bytes（`openssl rand -base64 32`） | **遺失＝收件資訊全部無法解密；無輪替機制**；見 §6 |
| `S3_ENDPOINT`、`S3_BUCKET`、`S3_ACCESS_KEY`、`S3_SECRET_KEY`、`S3_PUBLIC_BASE` | 必填 | `S3_REGION` 選填（R2 用 `auto`） |
| `NATIVE_ORIGINS`、`NATIVE_REDIRECT` | 要出原生殼才設 | 未設只允許 `APP_URL` |
| `GOOGLE_*`、`LINE_*` | 要啟用第三方登入才設 | redirect URI 為 `https://<API 網域>/api/v1/auth/oauth/{google,line}/callback`，須登錄於供應商後台（issue #6） |

- [ ] 每個值已放入 Dokploy 環境變數，並記錄「誰能讀、如何輪替、Dokploy 設定如何備份」。
- [ ] production 值與 dev 預設不同，且 staging／production 互不相同。
- [ ] 啟動煙霧測試：故意移除 `TURNSTILE_SECRET` 確認服務拒絕啟動。

## 4. Cloudflare 設定（DNS／CDN／WAF）

- [ ] `api.<域名>` 的 DNS 為 Tunnel（CNAME 至 `<tunnel>.cfargotunnel.com`，proxied）。
- [ ] 快取規則（README〈CDN 快取與受限清單〉）：
  - `/api/v1/public/*`：Edge TTL 選「Use cache-control header if present」，**不要**用 Cache Everything＋覆寫 TTL。
  - `/api/v1/restricted/*`：不套任何快取規則。
  - `/api/v1/*/events`（SSE）：不快取、不緩衝、不壓縮。
- [ ] WAF／速率限制規則；**對外封鎖或限流 `/readyz`**（它查 DB）；`/healthz` 可開放。
- [ ] Turnstile widget 建立，hostname 含前端網域。
- [ ] Email Service：網域驗證（SPF／DKIM／DMARC）通過；實際寄出 OTP 到外部信箱，未進垃圾信。
- [ ] 前端 Pages：`NUXT_PUBLIC_API_BASE`、`NUXT_PUBLIC_TURNSTILE_SITE_KEY` 為**建置時**變數（`npm run deploy` 缺 Turnstile key 會中止）。部署會取代現有 `wishly-web`，先確認其內容（疑似 demo）可被取代。

## 5. 部署順序

1. [ ] 空庫（或 staging）完整 migration 成功。
2. [ ] CI 綠燈（後端測試、clippy、覆蓋率、`mutate.sh all`、`image` job）。
3. [ ] 備份機制已設定且**至少成功備份並還原過一次**（§2.1）——**沒有可還原的備份不得開放真實使用者**。
4. [ ] 部署後端到 Dokploy，`/healthz`、`/readyz` 通過。
5. [ ] 設定 Tunnel 與 Cloudflare 路由、快取規則（§2.5、§4），驗證無法繞過 Tunnel。
6. [ ] 部署前端（正式模式，不是 `deploy:demo`）。
7. [ ] 立即執行 §7 驗證；失敗則回滾（§8）。

前端可先於後端上線（讀 `/restricted` 得 404 會退回舊路徑），後端上線後須用好友帳號實際開一次好友限定清單。

## 6. 營運與資料保護

- [ ] **`SHIPPING_ENC_KEY` 備份於兩個獨立位置（密碼管理器＋離線），且不與資料庫備份放在同一處**。同機部署下「攻擊者需同時取得 DB 與金鑰」的前提變弱：單一主機入侵即可同時取得兩者，因此備份尤其不可把金鑰與 dump 放在一起。
- [ ] 主機存取最小化：誰有 SSH、誰能進 Dokploy、誰能讀環境變數；Dokploy 登入啟用 2FA（若支援，**需實測**）。
- [ ] staff 帳號建立方式（`users.is_staff`）與初始管理員流程已定；`system_flags.read_only` 唯讀開關可操作（維護與事故時用）。
- [ ] 監控與告警（外部監控打 `/readyz`，不要只靠主機內部）：
  - 服務：5xx 比率、延遲 p95、`/readyz` 失敗。
  - 主機：磁碟使用率（含 WAL／備份）、記憶體、CPU、Postgres 連線數、容器重啟次數。
  - 業務：寄信佇列積壓、**逾期認領未釋放數量**（偵測 §0.6 的靜默停擺）、Tunnel 狀態。
  - 備份：**備份成功與時間新鮮度**（備份靜默失敗是最常見的事故）。
- [ ] 事故聯絡人與值班方式；VPS 單點故障的恢復手冊（§2.1）。

## 7. 部署後驗證（附證據）

| # | 驗證 | 方法 |
|---|---|---|
| 7.1 | 服務存活與 DB 連得上 | `curl https://<api>/healthz`；`/readyz`（從內網或允許來源） |
| 7.2 | 註冊／登入 OTP 實際收信 | 以真實信箱走 F1；確認 `Set-Cookie` 含 `Secure; HttpOnly; SameSite=Lax` |
| 7.3 | 建清單、訪客認領、防超賣 | 在 staging 走一遍；正式環境不跑會寫入大量資料的 E2E |
| 7.4 | 限流以真實用戶 IP 計 | 兩個不同 IP 各打超過額度，確認互不影響（`TRUSTED_PROXY` 與 `CF-Connecting-IP` 生效） |
| 7.5 | **API 不可繞過 Tunnel** | 對 VPS 公網 IP 直連 80／443／8080 全部失敗 |
| 7.6 | CDN 金絲雀 | `scripts/cdn-canary.mjs` exit 0 |
| 7.7 | 清單改為受限後立即清快取 | 公開清單先被快取，改成 friends，匿名請求立即得到 403／404 |
| 7.8 | 圖片上傳與讀取 | 上傳→經 `S3_PUBLIC_BASE` 顯示 |
| 7.9 | 檢舉＋Turnstile | 送出檢舉成功；偽造 token 得 403 |
| 7.10 | SSE 長連線 | `scripts/sse-probe.mjs` 跑 ≥ 40 分鐘，確認越過 30 分鐘上限會重連（完整驗收是 Spike S3：120 分鐘） |
| 7.11 | 背景工作實際在跑 | 建一筆短效期認領，確認到期後自動釋放 |
| 7.12 | OAuth | Google／LINE 實際登入一次（issue #6） |
| 7.13 | **備份還原演練** | 還原到暫存實例，跑對帳查詢，記錄實際 RTO |
| 7.14 | 換版期間的影響 | 在 Dokploy 重新部署時，記錄進行中請求的失敗數；確認客戶端重試後認領狀態一致 |
| 7.15 | 重啟自癒 | 手動 kill 容器，確認自動重啟、資料不遺失、Tunnel 恢復 |

## 8. 回滾

- [ ] Dokploy 保留前一個部署版本，可一鍵切回。
- [ ] Migration 僅向前相容（expand → migrate → contract）：**不要在同一版移除舊欄位**；回滾後端時資料庫已套用的 migration 必須與舊版程式相容，部署前逐一檢查新 migration。
- [ ] 緊急時以 `system_flags.read_only=true` 降為唯讀，保住資料再處理。
- [ ] 前端回滾：Pages 部署紀錄切回前一版。
- [ ] 資料層回滾（誤刪、壞 migration）：只能靠 §2.1 的備份，所以高風險 migration 前先手動做一次備份並記錄。

## 9. 與 Spike 的關係

S3 受測鏈路已因 D49 改為 **Cloudflare → Tunnel → VPS（Dokploy／Traefik）→ API**，原規格的 Cloud Run 不再適用。**這是驗收條件的變更，已記錄於 PRD，但 Spike 門檻（120 分鐘、0 次 >30 s 間隔、事件延遲 p50≤2 s／p95≤5 s／max≤10 s）不變。**
S2／S3 的前置就是本清單 §2–§5；完成 §7.1–§7.10 後，依 `git show 5c13bb9:docs/qa/spike-protocol.md` 的門檻執行（docs/qa 已移除，結果請另行記錄）。S3 必須在**最終的** Dokploy／Traefik／Tunnel 組態上測，換設定後要重測。

## 10. 未驗證的假設（本文件不聲稱已成立）

- Dokploy 內建備份的實際機制（是否 `pg_dump`、可否 PITR、排程最短間隔）、部署新版時的啟停順序、管理介面預設暴露方式——均**需實機確認**。
- Tunnel → Traefik → axum 這條路徑對 SSE 的緩衝、逾時與 `CF-Connecting-IP` 傳遞行為，無任何量測資料（即 Spike S3）。
- 後端在該 VPS 規格下的容量（併發認領、SSE 連線數、`image` 解碼對 CPU 的影響）無量測資料。
- `lib.rs` 的 CORS／Origin 檢查是否符合 `docs/04` 所述（非 GET 驗證 `Origin`）——未在本次查核中逐項驗證。
- 金流、電子發票、法遵（PRD P2 前 Go/No-Go）不在本清單範圍。
