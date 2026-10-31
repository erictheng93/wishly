# Wishly 願望清單（MVP）

建立願望清單、用連結分享，親友免登入即可瀏覽，登入後即可認捐品項（訪客僅能瀏覽，D50），並確保不會重複捐助（超賣防護）。

## 結構

| 路徑 | 內容 |
|---|---|
| `apps/web` | Nuxt 4 前端（Cloudflare Pages；`/s/[slug]` 為 SSR 分享頁，供 LINE / FB 預覽） |
| `apps/api` | Rust（axum + sqlx）後端 |
| `db/migrations` | PostgreSQL 16 + PostGIS 遷移（MVP 14 張表） |
| `docs/` | PRD、使用者流程、線框、資料 schema 與 API 契約（v0.4） |
| `scripts/cargo.sh` | 沒安裝 Rust 時，用 Docker 執行 cargo |

## 本機開發

### 安裝 Rust（建議，編譯比 Docker 快很多）

```sh
brew install rustup && rustup-init -y     # 或：curl https://sh.rustup.rs -sSf | sh
source "$HOME/.cargo/env"
cargo --version                            # 確認安裝成功
```

可選的加速設定：在 `apps/api/Cargo.toml` 加上

```toml
[profile.dev]
debug = "line-tables-only"                 # 縮短連結時間
```

可選工具（`cargo binstall` 只用來下載現成工具的執行檔，不會加速專案本身的編譯）：

```sh
cargo install cargo-binstall
cargo binstall sqlx-cli cargo-watch        # sqlx-cli：管理 migration；cargo-watch：存檔自動重啟
```

沒安裝 Rust 時，仍可用 `scripts/cargo.sh test`（Docker）執行，只是較慢。

```sh
docker compose up -d                 # PostGIS、MinIO（模擬 R2）、Mailpit（收 OTP 信：http://localhost:8025）

# 後端（啟動時自動套用 migration）
export DATABASE_URL=postgres://wishly:wishly@localhost:5432/wishly
export APP_ENV=dev   # 必填：未設即視為 production（缺機密 / 密鑰 <32 字元會拒絕啟動，見 apps/api/.env.example）
cargo run --manifest-path apps/api/Cargo.toml     # http://localhost:8080
# 沒有 Rust：scripts/cargo.sh test

# 前端
cd apps/web && npm install && npm run dev         # http://localhost:3000
```

## 登入方式管理

`/settings` 可綁定或解除 Email／Google／LINE，保留至少一種登入方式。LINE 無 Email 帳號可透過 OTP 補上信箱；新增／解除會寄送安全通知。註冊頁也提供 Google／LINE 入口。既有兩個帳號不搬移資料合併；已驗證 Email 的自動連結政策保持有效。

詳見[登入方式與 Email 政策](docs/account-login-methods.md)，包含無 Email 限制、解除後重新連結、頭像轉存、JWKS 快取與新增 API。新增 migration `0007_identity_linking.sql`，API 啟動時自動套用，無新增環境變數或依賴。

## 營運後台

`/admin` 提供統計、檢舉、清單與使用者管理、唯讀旗標；`/admin/orders` 為採購佇列、`/admin/wallets` 為點數管理、`/admin/audit` 可查詢稽核紀錄；`/admin/cash-refunds` 為現金退款審核、`/admin/wallet-debts` 為拒付欠款結案、`/admin/finance` 為金流後台（總覽、交易明細、付費用戶、渠道、風險與負債；儲值單的拒付爭議處理在交易明細分頁，舊路徑 `/admin/topups` 會導向）。使用者端現金退款在 `/me/refund`（由 `/me/wallet` 進入，只有儲值購買的點數可退）。

- 第一位 staff 需由資料庫設定 `users.is_staff = true`；之後可在後台設為／取消 staff。最後一位有效 staff 不可被停權、降權或自行刪帳號。
- 停權、staff 變更與營運解除綁定會撤銷現有登入；解鎖後需重新登入。停權保留清單、捐助與帳務紀錄，不會自動下架清單。
- 使用者支援 Email、名稱、handle、UUID 查詢，以及登入來源／帳號狀態篩選。停權、staff 變更、解除綁定與復原協助必須填寫原因並留下稽核。
- 解除 LINE／Google 綁定至少須保留一種登入方式，並更新聯絡 Email、寄送安全通知。復原協助僅向既有已驗證 Email 寄送密碼重設驗證信；無 Email 帳號須另行核驗身分，後台不能直接換綁。
- 新增 migration `0008_admin_users.sql`（`users.blocked_at`），由 API 啟動時套用，無新增環境變數。稽核日期依台北時區且包含結束日。

## 測試

`apps/web/qa/*.mjs` 是手動探索腳本（crawl、network、security、expiry、invariants 等），需手動啟動服務並準備資料；**不屬於 CI 自動測試**。可重複驗證的回歸測試以 `apps/api/tests/`、`apps/web/tests/` 與 `apps/web/e2e/` 為準。

後端一般測試也會驗證真實 SMTP 寄送，請先啟動 `docker compose up -d db mailpit`。`scripts/cargo.sh` 已設定容器連到宿主的 SMTP 與 Mailpit API；直接跑 cargo 預設使用 localhost。

CI 會執行 Rust fmt／clippy（warnings 視為錯誤）、依賴 audit 與覆蓋率檢查。前端用 `npm run test:coverage`；API 用 `sh scripts/coverage-api.sh`，門檻為已量測基準減 0.5 個百分點，基準不自動下調。Rust 固定 1.99.0、cargo-llvm-cov 固定 0.9.1、cargo-audit 固定 0.22.2；覆蓋率工具版本不符會拒絕。`web-audit` job 執行 `npm run audit:security`（原始 `npm audit --audit-level=high`，不設例外，保留 exit code 與完整 JSON），每週一排程也會重查。歷史範圍、更新基準流程及 mutation 證據記錄於 `git show 5c13bb9:docs/qa/test-gaps-audit.md`（文件已移除）；現行依賴修補政策以[安全 fork 說明](apps/web/patches/README.md)為準。

刪除已覆蓋的程式碼也會改變覆蓋率的分子與分母；若比例低於基準減 0.5pp，API gate 會變紅。刻意調整測試範圍或程式碼時，請重新量測並同步更新 baseline，勿將此結果視為 flaky。

前端的 `braces`／`node-forge` 由 `package.json` 的 `overrides` 換成[已修補的 fork](apps/web/patches/README.md)（固定 commit SHA）。回歸測試 `tests/dependencySecurity.test.ts` 驗證深層 glob 與惡意 RSA DigestAlgorithm 都被拒絕。

Mutation 可在 Mac／Linux 用 `scripts/mutate.sh --list` 選擇，例如 `scripts/mutate.sh cdn-account-delete`、`scripts/mutate.sh cdn-close-due`；需 Python 3、Rust 與可建立測試 DB 的 DATABASE_URL。runner 自動確認原版綠、停用單一行為後紅、還原後綠；CI 的獨立 `mutate` job 執行 `scripts/mutate.sh all` 並上傳 receipts／patch／logs（不擋 `publish-image`）。輸出保存於 `apps/api/target/mutations/`，請勿與相同 SQLx test path 的一般測試併跑。`python3 scripts/test_mutate.py` 不需要 DB，可驗證 runner 的分類與中斷還原。

```sh
cargo test --manifest-path apps/api/Cargo.toml    # 每個測試使用獨立資料庫（sqlx::test）
cd apps/web && npm run typecheck && npm run test:coverage && npm run build   # CI 的 web job 依序執行這三步
```

### 端對端測試（Playwright）

`apps/web/e2e/` 以真實的 API + Web + Postgres + Mailpit 跑瀏覽器流程（認捐一律以註冊會員身分進行，OTP 從 Mailpit 取，helper 為 `signInPage` / `memberClaim`；登入會員認捐、未登入「登入後認捐」導引與登入後導回、防超賣、心願眾籌（發點、贊助 / 撤回、達標、捐款者名單、採購與差額退回、轉投）、創建者註冊 / 登入 / 發佈、即時更新、驚喜模式、檢舉下架、LINE 內建瀏覽器、版面煙霧測試、儲值頁 `/me/topup`〔`topup.spec.ts`：登入走真後端，wallet / topups 以 `page.route` 模擬，攔截導向綠界的表單 POST 驗證，不連外〕、現金退款 `/me/refund`〔`refund.spec.ts`〕與營運三頁〔`admin-wallet-ops.spec.ts`：登入與 staff 權限走真後端，錢包 / 後台端點以 `page.route` 模擬，共用 `e2e/support/mockApi.ts`〕等）。

前置條件：
- `docker compose up -d db mailpit`（Postgres 在 5432、Mailpit 在 8025；OTP 信從 Mailpit HTTP API 讀取）。
- 已安裝 Rust（`cargo` 在 PATH 上）與 Node 22。第一次執行：`cd apps/web && npm ci && npx playwright install chromium`。
- 本機不需要安裝 `psql`：會改用 `docker compose exec db psql`（CI 則用本機 psql）。
- 不要設定 `GOOGLE_CLIENT_ID` / `LINE_CHANNEL_ID` 等憑證環境變數（「未設憑證」的登入案例依賴它們不存在）。

```sh
cd apps/web
npm run e2e            # 全部（desktop-chromium + mobile，最後執行 admin）
npx playwright test --project=admin --no-deps  # 只跑後台（含唯讀旗標，不與其他 E2E 同時執行）
npm run e2e:ui         # Playwright UI 模式
npm run e2e:headed     # 顯示瀏覽器
npx playwright test guest-journey --project=desktop-chromium   # 單一檔案（會員認捐旅程；login-to-claim 為未登入導引）
npx playwright show-report                                      # 開啟上次的 HTML 報告
```

隔離方式（不會干擾開發環境）：
- 資料庫 `wishly_e2e`：每次執行前由 `e2e/support/reset-db.mjs` DROP 後重建，API 啟動時自動套用 migrations；開發用的 `wishly` 資料庫不受影響。
- API 用 `127.0.0.1:8081`、Web 用 `127.0.0.1:3012`（`playwright.config.ts` 的 `webServer` 啟動，且不重用既有伺服器；連接埠被佔用會直接失敗）。Web 是 production build + node-server（`e2e/.app` 是獨立 buildDir 的 Nuxt layer，不會覆蓋開發中的 `.nuxt`）。
- 每個測試自建唯一資料（唯一 email、唯一清單），所以可平行（本機 3 workers、CI 2）。後端限流以身分計（email / user；舊訪客身分僅剩管理既有捐助），每 IP 的額度（檢舉 10/小時、登入失敗 30/15 分鐘）遠大於整個測試集的用量；後端不信任 `X-Forwarded-For`，所以測試也不靠它。
- 同一 email 的 OTP 有 60 秒間隔（登入 / 註冊 / 重設共用），需要連續寄第二封時，測試用 `skipOtpCooldown(email)` 把舊紀錄往前推。
- 測試初始 staff 權限由 `makeStaff(email)` 以 psql 設定 `users.is_staff`。後台 `admin` project 依賴其他 project 完成後才執行，避免唯讀旗標影響平行測試。

失敗時看 `apps/web/playwright-report/`（`trace: on-first-retry`，CI 會重試 2 次並把報告與 trace 上傳為 artifact）。

## 環境變數

- 後端：`DATABASE_URL`；其餘（SMTP、LINE Login 等）見 `apps/api/.env.example`
  - `BIND`：預設 `127.0.0.1:8080`（dev、production 皆是）。容器內部署需明確設 `BIND=0.0.0.0:8080`，且只讓 Cloudflare Tunnel / 內網可達。
  - `NATIVE_ORIGINS`：額外允許的 CORS 來源（逗號分隔），給 Tauri webview：`tauri://localhost`（macOS / iOS / Linux）、`http://tauri.localhost`（Windows / Android）。dev 預設全含；**production 未設就只允許 `APP_URL`**。原生殼不送 cookie，以 `X-Native-Client: 1` 登入取回 `session_token`，之後用 `Authorization: Bearer`（與 cookie 共用同一張 `sessions`）；只有 `Origin` 屬於 `NATIVE_ORIGINS` 才會發 `session_token`，網頁版 origin 即使送了 header 也拿不到。Tauri 殼在 `apps/web/src-tauri`（權杖存 OS keychain；Android 改存 App 私有目錄，見其 README），建置方式見其 `README.md`。
  - `NATIVE_REDIRECT`：原生殼 OAuth 完成後導回 App 的 deep link 基底（預設 `wishly://auth`）。流程：App 帶 `start?native=true&app_challenge=<S256>` 開系統瀏覽器 → callback 302 到 `wishly://auth?code=…`（失敗為 `?error=…`）→ App 以 `POST /auth/native/exchange {code, verifier}`（需 `X-Native-Client` 且 Origin ∈ `NATIVE_ORIGINS`）換 `session_token`；碼 60 秒、單次使用、只存雜湊。
  - `TRUSTED_PROXY`：`cloudflare` 或 `none`。dev 預設 `none`；**production 必須明確設定**，未設或其他值會拒絕啟動（不設會讓所有使用者看起來來自同一 IP，使每 IP 限流變成全域限流）。設 `cloudflare` 時信任 `CF-Connecting-IP`，所以 API 不可被直接存取，否則該標頭可被偽造；若 `BIND` 不是回送位址，啟動時會印出 WARN。
  - `TURNSTILE_SECRET`：production 必填；檢舉的 `turnstile_token` 會送 siteverify 驗證，失敗回 403 `FORBIDDEN`。dev/test 不設則略過驗證（接受任何 token，含前端的 `dev-bypass`）。`TURNSTILE_VERIFY_URL` 僅測試時覆寫。
  - `S3_ENDPOINT`、`S3_BUCKET`、`S3_ACCESS_KEY`、`S3_SECRET_KEY`、`S3_PUBLIC_BASE`：production 全部必填，缺漏啟動即 panic 並指出缺哪個；dev 預設本機 MinIO。`S3_REGION` 選填。儲存後端須支援原子 `If-Match` / `If-None-Match` 條件式 PUT 與 ETag，避免逾時上傳在刪除後恢復圖片；production 使用 Cloudflare R2：`S3_ENDPOINT=https://<account_id>.r2.cloudflarestorage.com`、`S3_REGION=auto`（程式以 path-style 簽名）、`S3_PUBLIC_BASE` 為綁定 bucket 的自訂網域；API Token 只給該 bucket 的 Object Read & Write。`incoming/` 前綴放尚未去 EXIF 的原圖，**不得對外公開**，見 `docs/deploy-backend-checklist.md` §2.2。
  - `CF_ZONE_ID`、`CF_CACHE_PURGE_TOKEN`：`TRUSTED_PROXY=cloudflare` 的 production **必填**（token 只需 Zone → Cache Purge 權限）。清單改成好友 / 指定 / 密碼 / 私人、封存、下架、刪帳號時，後端立即清除該清單公開網址的邊緣快取。 刪除個人圖片後也依完整物件路徑清除媒體快取（含 query string 變體）；`S3_PUBLIC_BASE` 的媒體網域須位於 `CF_ZONE_ID` 指定的同一個 zone。沒有自己 zone 的快速通道才可明確設 `CDN_PURGE=off` 略過。見下方〈CDN 快取與受限清單〉。
  - `ORGS_ENABLED`：公益組織 / 救災功能總開關，僅 `true` 或 `1` 為開；**production 預設關**（未通過法務前保持關閉），關閉時 `/orgs`、`/org-invites`、`/relief-events`、`/public/orgs`、`/public/relief-events`、`/admin/orgs|permits|relief-events` 回 404，`GET /public/status` 的 `orgs_enabled` 讓前端隱藏入口。dev/test 恆開。
  - `ECPAY_MERCHANT_ID`、`ECPAY_HASH_KEY`、`ECPAY_HASH_IV`：綠界金流憑證，**選填**；三個都設才啟用儲值，否則 `POST /wallet/topups` 與 `POST /webhooks/payments/{provider}` 回 503 `TOPUP_DISABLED`。dev/test 預設用綠界公開測試參數（以綠界文件為準）。`ECPAY_ENDPOINT`（預設 stage `https://payment-stage.ecpay.com.tw/Cashier/AioCheckOut/V5`，正式為 `https://payment.ecpay.com.tw/Cashier/AioCheckOut/V5`）；`ECPAY_RETURN_URL` 為我方 webhook 的對外完整網址（dev 預設 `http://localhost:8080/api/v1/webhooks/payments/ecpay`，production 未設時取 `API_BASE_URL` + `/api/v1/webhooks/payments/ecpay`），須在綠界後台可達。`ECPAY_PAYMENT_INFO_URL` 為取號通知（PaymentInfoURL，超商代碼 / 條碼 / ATM）的對外完整網址（dev 預設 `http://localhost:8080/api/v1/webhooks/payments/ecpay/info`，production 未設時取 `API_BASE_URL` + `/api/v1/webhooks/payments/ecpay/info`），同樣須可達。`ECPAY_QUERY_URL` 為入帳前二次確認用的 QueryTradeInfo 位址（stage `https://payment-stage.ecpay.com.tw/Cashier/QueryTradeInfo/V5`，正式 `https://payment.ecpay.com.tw/Cashier/QueryTradeInfo/V5`，以綠界官方文件為準）；未設時由 `ECPAY_ENDPOINT` 把 `/AioCheckOut/` 換成 `/QueryTradeInfo/` 推導，所以正式環境只要 `ECPAY_ENDPOINT` 設對即可。`ECPAY_QUERY_TIMEOUT_SECS`（預設 8）為查詢逾時，逾時或失敗時 webhook 回 503 由綠界重送。`ECPAY_CARD_3D_CONFIRMED`（`true` / `1` 才開，dev/test 恆開）：綠界預設不強制 3D，D41 要求信用卡儲值一律 3D；向綠界申請開通強制 3D 並確認後才設，未設時 `method=card` 回 422 `UNSUPPORTED`，`/public/status` 的 `topup_methods` 也不含 `card`。
  - `SHIPPING_ENC_KEY`：收件資訊加密金鑰，**production 必填**，必須是 base64 編碼的 32 bytes，否則拒絕啟動。dev / test 不設時使用固定的開發金鑰。詳見下方〈收件資訊加密金鑰〉。

### 收件資訊加密金鑰（`SHIPPING_ENC_KEY`）

眾籌品項達標後由平台代購寄出，所以受捐者的收件姓名、電話、地址會在應用層以 XChaCha20-Poly1305 加密後才存進資料庫（`shipping_addresses.*_enc`、`purchase_orders.shipping_address_snapshot`）。資料庫、備份、log 都看不到明文，實作在 `apps/api/src/sealed.rs`。

- **產生**：`openssl rand -base64 32`。每個環境各自產生一把，不要共用，也不要 commit 進 repo。
- **部署**：production 一定要設定這個變數，否則後端拒絕啟動。正式環境加入時機：在部署包含 migration `0004_crowdfund.sql` 的版本之前。
- **遺失 = 地址全部讀不回來**：金鑰遺失或被換掉之後，所有已存的收件資訊和採購單上的地址快照都無法解密，營運就無法寄貨（後台採購單的收件資訊會是空的）。請像資料庫密碼一樣保管，至少備份在兩個獨立的地方（例如密碼管理器加上離線備份）。
- **不要更換**：目前沒有金鑰輪替機制（`key_version` 固定為 1）。要輪替時須先實作「多把金鑰依 `key_version` 解密，加上背景重新加密」，不能直接替換環境變數的值。
- **洩漏時**：攻擊者必須同時拿到金鑰和資料庫才能讀到地址。若確認金鑰外洩，請先實作輪替再重新加密，並評估是否需要通知受影響的使用者。
- **也用於兩步驟驗證**：營運人員的 TOTP 密鑰（`users.totp_secret`）同樣以此金鑰加密。金鑰遺失時，已啟用的人無法通過驗證、也進不了後台，須由另一位營運人員在後台「使用者」重設兩步驟驗證；若沒有其他營運人員，直接在資料庫執行 `UPDATE users SET totp_secret=NULL, totp_enabled_at=NULL, totp_last_step=NULL WHERE id='<uuid>';`。
- 前端：`NUXT_PUBLIC_API_BASE`（預設 `http://localhost:8080`）；`NUXT_PUBLIC_TURNSTILE_SITE_KEY`（檢舉用 Cloudflare Turnstile；未設定時檢舉送 `dev-bypass`，僅限本機 mock；**正式部署必填**，`npm run deploy` 缺少它會直接中止，`deploy:demo` 不受影響）
- 前端：`NUXT_PUBLIC_SUPPORT_EMAIL`（客服信箱，建置時寫入，預設 `support@wishly.tw`）；顯示在頁尾、條款、隱私權政策、設定頁，以及停權、錢包凍結、清單下架等「請聯絡客服」提示旁（`components/SupportLink.vue`，已登入時信件內文自動帶帳號 ID）

## 部署

前端部署到 Cloudflare Pages，有兩種，差別在「建置時」的環境變數（`wrangler pages deploy` 不支援 `--config`，所以不用第二份 wrangler 設定）：

| 指令（在 `apps/web`） | 建置時設定 | 行為 |
|---|---|---|
| `npm run deploy`（正式） | `NUXT_DEMO` 清空；API 預設 `https://api.wishly.tw` | 分享連結走真實 API，`/s/demo` 不會有假資料 |
| `npm run build:native`（Tauri 前端） | `NUXT_NATIVE=1`、`NUXT_PUBLIC_SITE_URL`（分享連結網域，預設 `https://wishly.tw`）、`NUXT_PUBLIC_API_BASE` | 純 SPA 靜態檔 `.output/public`（無 SSR、無 service worker、隱藏第三方登入）；Tauri 的 `frontendDist` 指向它。尚未包 Tauri 專案 |
| `npm run deploy:demo`（示範） | `NUXT_DEMO=1`、`NUXT_PUBLIC_API_BASE=` | 僅 `/s/demo` 有同源假資料，其餘分享連結失敗；**不要給真實使用者** |

- 範例清單（landing「看範例」，`/s/demobirth0`、`/s/demogift00`、`/s/demowedd00`）是真實 API 的 public 清單：部署後在正式 DB 執行一次 `psql "$DATABASE_URL" -f db/seed/demo.sql`（可重複執行）。它們不出現在探索頁與後台分析。
- 靈感頁 / 品牌館的模擬資料（10 個品牌、12 件精選商品，`p01`..`p12`，價格為示意）：部署後同樣在正式 DB 執行一次 `psql "$DATABASE_URL" -f db/seed/catalog.sql`（可重複執行）。真實商品上架後到後台 `/admin/catalog` 把這些品牌與商品下架（ref 不可刪，已加入清單的品項仍對得上）。


- 正式 API 網域不同時，建置前設 `NUXT_PUBLIC_API_BASE=https://...`，或改 `nuxt.config.ts` 預設值。
- `wrangler.jsonc` 刻意不放 `vars`，避免執行期變數蓋掉建置設定。
- 本機驗證：`npm run build` 後 `npx wrangler pages dev dist`（示範用 `npm run build:demo`）。


### CDN 快取與受限清單

分享頁資料分兩條路徑，**受限清單的內容永遠不會出現在可快取的路徑上**：

| 路徑 | 讀身分 | 回應 | CDN |
|---|---|---|---|
| `GET /api/v1/public/wishlists?page=&type=` | 不讀 cookie | 探索頁：所有人的公開清單（public、上線、有品項、未驚喜鎖定），每頁 24 筆 | 可快取（`public, s-maxage=10, stale-while-revalidate=60`） |
| `GET /api/v1/public/wishlists/{slug}` | 不讀 cookie | public 的內容；其餘只回不含內容的 403 / 404 | 可快取（`public, s-maxage=10, stale-while-revalidate=60`） |
| `GET /api/v1/restricted/wishlists/{slug}` | `ws_session` | 依可見性判斷 | 永不快取（`private, no-store`） |

- 所有非 `public` 的回應另帶 `Cloudflare-CDN-Cache-Control: no-store` 與 `CDN-Cache-Control: no-store`。
- **Cloudflare 後台設定**：`/api/v1/public/*` 的 Edge TTL 用「Use cache-control header if present」，**不要**用「Cache Everything + 覆寫來源 TTL」；`/api/v1/restricted/*` 不要套任何快取規則。
- **部署順序**：先前端、再後端（前端讀 `/restricted` 拿到 404 時會退回舊路徑，所以前端可單獨先上）。後端上線後用好友帳號實際開一次好友限定清單確認。
- **金絲雀**：用營運帳號建一份 friends 金絲雀清單、另一個帳號加為好友，部署後（或排程）跑：

  ```sh
  API_BASE=https://api.wishly.tw CANARY_SLUG=<friends 清單> CANARY_FRIEND_COOKIE='ws_session=<好友 session>' \
  CANARY_PUBLIC_SLUG=<public 清單> node scripts/cdn-canary.mjs   # 失敗 exit 1
  ```

  它模擬「好友先載入、陌生人再開同一網址」，檢查匿名請求拿不到清單內容且不是快取命中。

### 臨時公開測試（本機後端 + 快速通道 + Pages 預覽分支）

用來在手機或 LINE 上測試本機的後端。**這會把你電腦上的 API 經臨時網址公開**，請用獨立的資料庫與隨機機密，測完就關。

1. 後端：用獨立資料庫、隨機的 `OTP_PEPPER` / `OAUTH_SECRET` / `UNSUB_SECRET`，`TRUSTED_PROXY=cloudflare`、`CDN_PURGE=off`（快速通道沒有自己的 zone）、`BIND=127.0.0.1:8090`、`APP_URL=https://qa.wishly-web-5pd.pages.dev`。
2. 通道：`cloudflared tunnel --url http://127.0.0.1:8090`，取得 `https://xxx.trycloudflare.com`。
3. 前端：`NUXT_API_PROXY=https://xxx.trycloudflare.com npm run deploy:preview`（部署到 `qa` 預覽分支，不動正式網址；前端以同源代理連後端，cookie 才是第一方）。
4. 限制：**快速通道不支援 SSE**，即時更新會自動退回每 15 秒輪詢；通道網址每次重開都會變，需要重新部署。要驗證 SSE 需使用有網域的具名通道。

## 開發順序

**P0 上線阻擋（2026-10-12 查核）：S1–S3 尚無可追溯的真機結案證據，均未驗收；補測與技術決策結案優先於新功能。** 見 `git show 5c13bb9:docs/qa/spike-status.md`（文件已自 repo 移除）。

0. 骨架與 CI → 1. 技術原型 S1–S3（真實 Pages 帳號、LINE 預覽 / 內建瀏覽器 cookie / SSE，見 `docs/03` 第 14 節）
→ 2. 垂直切片：公開分享頁 → 訪客捐助 → 創建者登入與編輯 → 進度儀表板 / 驚喜模式 / 即時更新 → MVP 附加功能
→ 3. 封閉測試 8 週，依 PRD 的 go / no-go 標準決定是否進入第二期。
