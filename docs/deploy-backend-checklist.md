# 後端部署檢查清單（apps/api → 正式環境）

目標鏈路（PRD D48 / 技術選型）：Cloudflare（DNS／CDN／WAF）→ GCP Cloud Run asia-east1 → Cloud SQL（Postgres 16）＋ Cloudflare R2。前端為 Cloudflare Pages。
本文件只列「上線前必須成立的條件」，每項附驗證方式。勾選請附日期與證據（指令輸出、截圖、commit），否則視為未完成。

> 依據：`apps/api/src/config.rs`、`apps/api/.env.example`、README〈環境變數〉〈部署〉、`docs/01-prd.html` 技術選型、`docs/04` 內部端點。
> 查核日期 2026-10-15。**§0 是 repo 內已確認的缺口，不補上就無法部署。**

---

## 0. Repo 內已確認的缺口（先處理）

| # | 缺口 | 影響 | 建議 |
|---|---|---|---|
| 0.1 | **沒有 Dockerfile／容器建置**（`find` 全 repo 無 Dockerfile） | Cloud Run 無法部署；CI 也沒有 build image 步驟 | 新增多階段 Dockerfile（`rust-toolchain.toml` 版本、`cargo build --release`、非 root、最終 image 只含 binary）；`BIND=0.0.0.0:8080` |
| 0.2 | **沒有健康檢查端點**（`grep -i health apps/api/src` 無結果） | Cloud Run startup／liveness probe、外部監控無從探測 | 新增 `GET /healthz`（行程存活）與 `/readyz`（含 `SELECT 1`）；注意 `read_only` middleware 不應擋 GET |
| 0.3 | **背景工作跑在行程內**（`main.rs` 的 `notify::spawn_worker`、`ratelimit::spawn_cleanup`），且 `docs/04` 規劃的 `POST /internal/jobs/tick`（Cloud Scheduler）**未實作** | Cloud Run 預設「請求期間才配置 CPU」：沒有請求時 worker 會被凍結 → 逾期認領不釋放、清單不自動結束、寄信佇列不送 | 二選一並寫進決策：(a) 服務設 `min-instances=1` ＋「CPU always allocated」；(b) 實作 `/internal/jobs/tick`＋Cloud Scheduler OIDC，行程內任務改由它觸發 |
| 0.4 | 多實例同時跑 `spawn_cleanup`／`spawn_worker`：是否可重複執行未在文件中證明 | 擴縮到 >1 實例時可能重複寄信或重複處理 | 部署前確認 `expire_due`、`close_due`、`remind_closing`、`tick_funding`、寄信領取皆冪等或有 `SKIP LOCKED`／advisory lock；否則先固定 `max-instances=1` 並記錄 |
| 0.5 | `docs/domain-names.md` 品牌網域尚未定案；程式與 README 預設 `wishsync.tw`、`api.wishsync.tw` | 網域未定就無法簽 OAuth redirect、Email 驗證網域、Turnstile hostname | 先定網域再做 §3、§5 |

---

## 1. 前置決策（需要產品／負責人拍板）

- [ ] 正式網域與子網域規劃：前端（如 `app.<域名>`）與 API（如 `api.<域名>`）**必須同一 registrable domain**。原因：`ws_session` 為 `SameSite=Lax` 且不帶 `Domain`，前端跨站（例如 `*.pages.dev` 對 `api.<域名>`）會讓登入 cookie 失效。
- [ ] 環境清單：staging 與 production 各自獨立 DB、R2 bucket、機密（PRD：備份與環境）。是否先只建 staging？
- [ ] 後台排程方案（§0.3 a／b）。
- [ ] 後端費用上限與 `max-instances`；Cloud SQL 規格。

## 2. 基礎設施

### 2.1 資料庫（Cloud SQL Postgres 16）
- [ ] 版本 16+；`DATABASE_URL` 走私網或 Cloud SQL connector，不開公網 0.0.0.0/0。
- [ ] 連線數：行程 `max_connections(10)`（`main.rs`）× 實例數 ≤ Cloud SQL 上限（留管理連線餘裕）。
- [ ] 啟動時自動 `sqlx::migrate!`（`main.rs:22`）：確認 DB 帳號有 DDL 權限，或改為獨立 migration job（PRD：Cloud Run Job，expand → migrate → contract）。
- [ ] 目前 migrations 為 `0001`–`0006`；首次部署前在**空資料庫**完整跑一次並記錄結果。
- [ ] 每日備份＋PITR 7 天已啟用；**做過一次還原演練**（PRD：RPO ≤ 5 分鐘、RTO ≤ 1 小時）。
- [ ] 編碼 UTF-8；migration 用到的函式與擴充（如 `uuid_v7()`）在 Cloud SQL 上可建立——以空庫完整跑 migration 驗證，必要時確認該擴充在 Cloud SQL 支援清單內。

### 2.2 物件儲存（Cloudflare R2）
- [ ] bucket 建立；CORS 允許前端網域 PUT（presigned 直傳）。
- [ ] `S3_PUBLIC_BASE` 為對外可讀的網域（自訂網域經 CDN）。
- [ ] 存取金鑰僅限該 bucket 的讀寫；與 staging 分開。
- [ ] 物件版本／生命週期（PRD：保留 30 天）。
- [ ] 驗證：實際上傳一張圖，經 `S3_PUBLIC_BASE` 在瀏覽器開得到；頭像轉存（issue #6）能寫入。

### 2.3 Cloud Run
- [ ] 區域 asia-east1；image 來自 Artifact Registry（§0.1）。
- [ ] `BIND=0.0.0.0:8080`；容器 port 一致。
- [ ] **Request timeout 調高**：預設 300s，而 SSE 每條連線上限 30 分鐘（`dashboard.rs`）。設為 ≥ 1800s（上限 3600s），否則 S3 會在 5 分鐘被切斷。
- [ ] 併發（concurrency）：SSE 長連線佔用併發名額，依預期同時在線人數設定，並與 `max-instances` 一起估算。
- [ ] CPU／最小實例依 §0.3 決策。
- [ ] 服務帳號最小權限；機密用 Secret Manager 注入，不寫進 image 或 repo。
- [ ] **API 不可被繞過 Cloudflare 直接存取**（`TRUSTED_PROXY=cloudflare` 信任 `CF-Connecting-IP`，直連可偽造而繞過限流）。做法：Cloud Run 限制入口＋Cloudflare Tunnel／Authenticated Origin Pulls／Cloud Armor 只放行 Cloudflare IP 段；README 的 `bind_warning` 在非回送位址仍會印 WARN，需確認這是預期並記錄放行理由。
- [ ] 日誌：確認 `tracing` 輸出到 Cloud Logging，且**不含**機密、收件資訊明文、OTP。

## 3. 環境變數與機密（production 缺漏即拒絕啟動）

`APP_ENV` 未設視為 production（fail closed）。以下全數必填，缺漏會 panic 並指出變數名。

| 變數 | 要求 | 備註 |
|---|---|---|
| `APP_ENV` | `production` | 明確設定 |
| `DATABASE_URL` | 必填 | |
| `BIND` | `0.0.0.0:8080` | 容器內 |
| `APP_URL` | 前端正式網址 | CORS 只允許此來源（並帶憑證） |
| `API_BASE_URL` | API 正式網址 | |
| `OTP_PEPPER`、`OAUTH_SECRET`、`UNSUB_SECRET` | 各自獨立、≥32 字元（`openssl rand -hex 32`） | 更換 `UNSUB_SECRET` 會使既有退訂連結與清單存取權杖失效 |
| `CF_ACCOUNT_ID`、`CF_EMAIL_API_TOKEN`、`MAIL_FROM` | 必填 | `MAIL_FROM` 網域須已在 Cloudflare Email Service 驗證 |
| `TRUSTED_PROXY` | `cloudflare`（或明確 `none`） | 不設啟動失敗 |
| `CF_ZONE_ID`、`CF_CACHE_PURGE_TOKEN` | `TRUSTED_PROXY=cloudflare` 時必填 | token 只給 Zone → Cache Purge |
| `TURNSTILE_SECRET` | 必填 | 與前端 `NUXT_PUBLIC_TURNSTILE_SITE_KEY` 成對，hostname 需包含前端網域 |
| `SHIPPING_ENC_KEY` | base64 的 32 bytes（`openssl rand -base64 32`） | **遺失＝收件資訊全部無法解密；無輪替機制**，見 §6 |
| `S3_ENDPOINT`、`S3_BUCKET`、`S3_ACCESS_KEY`、`S3_SECRET_KEY`、`S3_PUBLIC_BASE` | 必填 | `S3_REGION` 選填（R2 用 `auto`） |
| `NATIVE_ORIGINS` | 要出原生殼才設 | 未設只允許 `APP_URL` |
| `NATIVE_REDIRECT` | 原生殼才需要 | 預設 `wishsync://auth` |
| `GOOGLE_CLIENT_ID/SECRET/REDIRECT_URI`、`LINE_CHANNEL_ID/SECRET/LINE_REDIRECT_URI` | 要啟用第三方登入才設 | redirect URI 須為 `https://<API 網域>/api/v1/auth/oauth/{google,line}/callback`，並登錄在供應商後台；對應 issue #6 |

- [ ] 上表每個值已放入 Secret Manager，並記錄「誰能讀、輪替方式」。
- [ ] production 值與 dev 預設（`dev-pepper` 等）不同，且 staging／production 互不相同。
- [ ] 啟動煙霧測試：故意移除 `TURNSTILE_SECRET` 確認服務拒絕啟動（驗證 fail-closed 在目標環境成立）。

## 4. Cloudflare 設定（DNS／CDN／WAF）

- [ ] API 網域 DNS 為 proxied（橘雲），SSL/TLS 模式 Full (strict)。
- [ ] 快取規則（README〈CDN 快取與受限清單〉）：
  - `/api/v1/public/*`：Edge TTL 選「Use cache-control header if present」，**不要**用 Cache Everything＋覆寫 TTL。
  - `/api/v1/restricted/*`：不套任何快取規則。
  - `/api/v1/*/events`（SSE）：不快取、不緩衝（確認未開 Rocket Loader／回應壓縮造成緩衝）。
- [ ] WAF／速率限制規則（PRD：限流用 Cloudflare 規則＋服務內層）。
- [ ] Turnstile widget 建立，hostname 含前端網域，取得 site key 與 secret。
- [ ] Email Service：網域驗證（SPF／DKIM／DMARC）通過；實際寄出一封 OTP 到外部信箱，未進垃圾信。
- [ ] 前端 Pages：`NUXT_PUBLIC_API_BASE`、`NUXT_PUBLIC_TURNSTILE_SITE_KEY` 為**建置時**變數（`npm run deploy` 缺 Turnstile key 會中止）。部署時蓋掉現有 `wishsync-web`，先確認其內容（疑似 demo）可被取代。

## 5. 部署順序

1. [ ] 空庫（或 staging）完整 migration 成功。
2. [ ] 後端 image build＋掃描（`cargo audit`／依賴檢查）通過；CI（`.github/workflows/ci.yml`）綠燈：後端測試 132 通過、`mutate.sh all` 全 killed。
3. [ ] 部署後端到 Cloud Run，`/healthz` 通過（§0.2）。
4. [ ] 設定 Cloudflare 路由與快取規則（§4），確認 API 只能經 Cloudflare 存取（§2.3）。
5. [ ] 部署前端（正式模式，不是 `deploy:demo`）。
6. [ ] 立即執行 §7 驗證；失敗則回滾（§8）。

順序依據：前端讀 `/restricted` 得到 404 會退回舊路徑，所以前端可先上；後端上線後必須用好友帳號實際開一次好友限定清單。

## 6. 營運與資料保護

- [ ] `SHIPPING_ENC_KEY` 備份於**兩個獨立位置**（密碼管理器＋離線）；備份位置寫入內部文件，不寫入 repo。
- [ ] 資料庫＋金鑰分開保管（攻擊者需同時取得兩者才能讀到地址）。
- [ ] 後台 staff 帳號建立方式（`users.is_staff`）與初始管理員流程已定；`system_flags.read_only` 唯讀開關可操作（維護／事故時用）。
- [ ] 監控與告警：5xx 比率、延遲 p95、DB 連線數、Cloud Run 實例數、SSE 同時連線數、寄信佇列積壓、逾期認領未釋放數量。
- [ ] 事故聯絡人與值班方式。

## 7. 部署後驗證（附證據）

| # | 驗證 | 方法 |
|---|---|---|
| 7.1 | 服務存活與 DB 連得上 | `curl https://<api>/healthz`、`/readyz` |
| 7.2 | 註冊／登入 OTP 實際收信 | 以真實信箱走 F1；確認 `Set-Cookie` 含 `Secure; HttpOnly; SameSite=Lax` |
| 7.3 | 建清單、訪客認領、防超賣 | Playwright E2E 對正式環境不可跑（會寫資料）；改用 staging，或手動走一遍 |
| 7.4 | 限流以真實用戶 IP 計 | 兩個不同 IP 各打超過額度，確認互不影響（`TRUSTED_PROXY` 生效） |
| 7.5 | CDN 金絲雀 | `scripts/cdn-canary.mjs`（README〈CDN 快取與受限清單〉）exit 0 |
| 7.6 | 清單改為受限後立即清快取 | 公開清單先被快取，改成 friends，匿名請求立即得到 403／404 |
| 7.7 | 圖片上傳與讀取 | 上傳→經 `S3_PUBLIC_BASE` 顯示 |
| 7.8 | 檢舉＋Turnstile | 送出檢舉成功；偽造 token 得 403 |
| 7.9 | SSE 長連線 | `scripts/sse-probe.mjs` 跑 ≥ 40 分鐘，確認越過 30 分鐘上限會重連（完整驗收是 Spike S3：120 分鐘） |
| 7.10 | 背景工作實際在跑 | 建一筆短效期認領，確認到期後自動釋放（驗證 §0.3 決策） |
| 7.11 | OAuth | Google／LINE 實際登入一次（issue #6） |
| 7.12 | 備份還原演練 | 從 PITR 還原到臨時實例，確認資料可讀 |

## 8. 回滾

- [ ] Cloud Run 保留前一個 revision，可一鍵切流量。
- [ ] Migration 僅向前相容（expand → migrate → contract）：**不要在同一版移除舊欄位**；回滾後端時 DB 已套用的 migration 必須與舊版程式相容，部署前逐一檢查新 migration。
- [ ] 緊急時以 `system_flags.read_only=true` 降為唯讀，保住資料再處理。
- [ ] 前端回滾：Pages 部署紀錄切回前一版。

## 9. 與 Spike 的關係

S2／S3 的前置就是本清單 §2–§5：沒有真實後端、具名網域與 Cloud Run 逾時設定，S3 無法在規格指定的鏈路上驗收。完成本清單 §7.1–§7.9 後，依 [spike-protocol](qa/spike-protocol.md) 執行，結果填入 [spike-decisions](qa/spike-decisions.md)。

## 10. 未驗證的假設（本文件不聲稱已成立）

- `lib.rs` 的 CORS／Origin 檢查是否符合 `docs/04` 所述（非 GET 驗證 `Origin`）——未在本次查核中逐項驗證。
- 後端對 Cloud Run 實際行為（冷啟動時間、記憶體用量、SSE 併發上限）無任何量測資料。
- 金流、電子發票、法遵（PRD P2 前 Go/No-Go）不在本清單範圍；上線前另有阻擋項，見 `TODO.md` 與 PRD。
