# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

Wishly：願望清單，用連結分享，訪客僅能瀏覽、認捐需登入（D50），防超賣。文件與程式註解以繁體中文為主。完整設定與部署細節見 `README.md`；產品/契約規格在 `docs/`（PRD、流程、線框、`04-data-schema-api-contract.html` 的 API 契約，程式註解常以「契約 x.x」「D48」引用）。

## 指令

```sh
docker compose up -d                       # PostGIS、MinIO（模擬 R2）、Mailpit（OTP 信 http://localhost:8025）

# 後端 apps/api（Rust, axum + sqlx）；啟動時自動套用 db/migrations
export DATABASE_URL=postgres://wishly:wishly@localhost:5432/wishly
export APP_ENV=dev                         # 必設：未設視為 production，缺機密會拒絕啟動
cargo run --manifest-path apps/api/Cargo.toml          # :8080
cargo test --manifest-path apps/api/Cargo.toml         # sqlx::test，每個測試獨立資料庫（需 DB 在跑）
cargo test --manifest-path apps/api/Cargo.toml --test claims <name>   # 單一測試檔 / 單一測試
scripts/cargo.sh test                      # 沒裝 Rust 時用 Docker

# 前端 apps/web（Nuxt 4）
cd apps/web && npm install && npm run dev  # :3000
npm run typecheck && npm test && npm run build   # CI 依序執行；單一測試：npx vitest run tests/errors.test.ts
node mocks/server.mjs                      # 訪客端契約 mock，:8080（slug: demo/closed/gone/boom/sseoff），不需後端/DB
```

## 架構

**後端**（`apps/api/src`，lib `wishly_api`）：每個模組各自提供 `routes()`，由 `lib.rs::app()` 合併到 `/api/v1`；新增模組只改那裡。全域 middleware：`read_only`（`system_flags.read_only` 為真時非安全方法回 503，auth/admin system-flags 例外）與 CORS（只允許 `APP_URL` 並帶憑證）。`config.rs` 是全域單例（`config::init/get`），production 缺機密、`TRUSTED_PROXY`、S3 設定會啟動即失敗。

- **兩種身分**：創建者用 cookie `ws_session`（`session::CurrentUser` extractor，DB 只存 SHA-256）；`POST /items/{id}/claims` 須登入（`ws_session` 或 Bearer），未登入 401（D50）；舊訪客權杖（`X-Guest-Token` 優先、其次 `ws_guest` cookie，`guest.rs`，`Actor::Guest|User`）僅供管理既有訪客認捐，不可新增或增量。
- **捐助（`claims.rs`）**：核心是超賣防護。鎖序固定 `wishlist_items`（條件式 UPDATE / FOR UPDATE）→ `claims`，改動時不可顛倒。寫入需 `Idempotency-Key`（UUID），`idempotency.rs` 的占位與業務寫入在同一交易，失敗 rollback 不留痕；重放回應帶 `Idempotency-Replayed`。逾期捐助會釋放，另有 `ratelimit.rs`（DB 計數，背景清理）。
- **心願眾籌（P2-A）**：`points.rs` 是唯一能改 `point_wallets.balance` 的地方（`post` 同交易寫 `point_ledger`，帳本只增不改）；`release` / `refund_partial` 內部先鎖錢包，所以要在動 `wishlist_items` 之前呼叫。鎖序 wishlists（FOR SHARE）→ point_wallets（依 id）→ wishlist_items → contributions。點數來源：後台人工發放，以及儲值 `topup.rs`（綠界：`POST /wallet/topups` 建單、`POST /webhooks/payments/ecpay` 驗簽後**在交易外**以 QueryTradeInfo 二次確認、再開交易 `FOR UPDATE` 入帳；`/info` 收取號通知寫 `payment_info`；外部 HTTP 不可搬進持鎖交易，D56）。點數分批（D55，`point_lots`）：`post` 依 kind 建批次（topup 可退現金、人工發放為 adjustment 不可退現金），扣點順序 reward_bonus → gift_card → adjustment → topup 並寫 `contribution_lot_allocations`，`release` / `refund_partial` 依分配逆序退回原批次；不變式 Σ lots.remaining = balance（`assert_points_reconciled` 會檢查），lots 靠 wallet 列鎖，不另鎖。拒付走營運入口 `POST /admin/topups/{id}/dispute`（`wallet_ops::dispute_open / dispute_won / charge_back`，鎖序 topups → 錢包；D57），供應商爭議 webhook 無官方格式、只留 501 接口。收件資訊以 `sealed.rs` 加密，金鑰 `SHIPPING_ENC_KEY` 遺失就無法解密（見 README）。
- **併發防護的回歸測試**：改動鎖序或併發相關 SQL（`FOR UPDATE` / `FOR SHARE`、`ON CONFLICT`）時，須在 `scripts/mutate.py` 補對應 mutant，並確認 `scripts/mutate.sh all` 全數 killed（還原後 SHA 一致）。
- **清單可見性**：只有 public / friends / selected / private 四種，新清單預設 friends（0016 移除 link、password）；`GET /public/wishlists` 是訪客可看的探索頁（只列 public、上線、有品項）。判斷集中在 `access.rs`（`check_wishlist`）。`GET /public/wishlists/{slug}` 會被 CDN 快取，**不可讀 cookie**，只回 public 內容；受限內容（friends＝所有好友、selected＝被選中的好友/摯友）走 `/restricted/wishlists/{slug}`（永遠 no-store）。清單不再可公開提供時呼叫 `cdn::purge_wishlists`。
- **公益組織與救災據點（0041）**：`orgs.rs`（申請 / 審核狀態機、文件、許可，`require_role` / `member_role`）、`org_admin.rs`（staff 審核、許可撤銷）、`org_members.rs`（成員、email 邀請）、`org_sites.rs`（據點建立、`lock_event`）、`org_receipts.rs`（實收）、`relief_events.rs`（事件、合併、下架、`permit_expiry_tick`）。據點就是 `type='relief'` 的 wishlist，品項可 quantity 或 crowdfund；擁有權**不可寫 `owner_id = 使用者`**，一律用 SQL 函式 `can_manage_wishlist(owner_id, org_id, uid)`（組織清單看 org_members 的 owner / coordinator，個人清單看 owner_id）。鎖序：`organizations`（FOR UPDATE / FOR SHARE）→ `org_members` / `org_invites` / `org_permits`；跨事件為 `organizations` → `relief_events`（FOR SHARE）→ `wishlists`；事件合併一次鎖兩列，依 id 由小到大。公開端點 `/public/relief-events*`、`/public/orgs/{slug}` 會被 CDN 快取、不可讀 cookie，且查詢一律檢查組織仍為 verified（`public.rs` 的 `LIVE_SITE`）；改動會影響公開頁的操作要呼叫 `cdn::purge_events` / `purge_org` / `purge_wishlists`。帳號刪除會把據點移交給另一位有效的 owner / coordinator，沒有人可接就關閉；最後一位 owner 離開時最早加入的 coordinator 升為 owner。據點恢復營運（site_status 回 active）與上線一樣要重驗組織與許可（trigger 也看 site_status）；PATCH 的 UPDATE 會比對讀到的 site_status / event_id，被並行撤銷、下架、合併改過就回 409。事件取消下架不會恢復已暫停的據點。整組功能由 `ORGS_ENABLED` 總開關控制（production 預設關、dev/test 開），關閉時上述路由由 `lib.rs::orgs_gate` 回 404，`/public/status` 帶 `orgs_enabled` 供前端隱藏入口。
- 背景工作在 `main.rs` 啟動：`notify::spawn_worker`（寄信）、`ratelimit::spawn_cleanup`、`relief_events::spawn_permit_expiry`（每小時跑 `permit_expiry_tick`：許可到期的上線據點自動暫停並通知 owner）。錯誤統一為 RFC 9457 problem+json（`error.rs`，含 `code` 與 `errors[]`）。
- 測試在 `apps/api/tests/*.rs`（依功能切片命名），皆用 `#[sqlx::test(migrations = "../../db/migrations")]`。
- DB：PostgreSQL 16 + PostGIS，`db/migrations/000N_*.sql` 依序累加。

**前端**（`apps/web`）：
- `/s/[slug]` 為 SSR 分享頁（LINE/FB 預覽）；創建者頁面（dashboard、lists、settings、admin、登入等）在 `nuxt.config.ts` 的 `routeRules` 設為 `ssr: false`（靠 cookie 驗證），新增創建者路由要加進該清單。
- API 在不同網域：`useApi()`（創建者，`credentials: 'include'`，把錯誤正規化為 `CreatorApiError`，401 自動導向登入）與 `useGuest()`（訪客權杖）。即時更新 `useWishlistEvents` 用 SSE，連續失敗 2 次改輪詢。
- 原生殼（Tauri）：`npm run build:native`（`NUXT_NATIVE=1`，建置時常數 `__NATIVE__`）產出純 SPA。webview 跨站不送 cookie，所以登入帶 `X-Native-Client` 取 `session_token` 後走 `Authorization: Bearer`（`utils/native.ts`、後端 `session::session_token`）；`session_token` 只在請求 `Origin` 屬於 `NATIVE_ORIGINS` 時發出（`auth::native_client`），網頁 origin 拿不到；權杖存 OS keychain（`src-tauri` 的 `session_token_*` 指令），記憶體持有；CORS 來源由 `NATIVE_ORIGINS` 控制。新增直接 `$fetch` 的受保護請求要帶 `nativeHeaders()`。
- 部署 Cloudflare Pages。`NUXT_DEMO` 與 `NUXT_PUBLIC_API_BASE` 是**建置時**變數：`npm run deploy`（正式）與 `deploy:demo`（只有 `/s/demo` 假資料，`server/routes/api/v1/public/...` 僅此時啟用，勿給真實使用者）；`wrangler.jsonc` 刻意不放 `vars`。
- 樣式為「紅白實驗室」主題（白底、紅 #c8102e、直角、警示斜紋進度條）；token 在 `assets/tokens.css`，參考 `design/themes.html` 款式二。
