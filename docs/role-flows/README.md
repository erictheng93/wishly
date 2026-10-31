# 角色流程實機模擬（2026-10-29）

以三種角色實際「點」一遍 Wishly：設計各角色會走的操作流程，再用真實瀏覽器（Playwright，Chromium）對本機完整服務逐步執行，記錄每一步的結果、截圖與問題。

| 文件 | 內容 |
|---|---|
| [01-user-recipient.md](01-user-recipient.md) | 用戶・受捐者（開清單的人）流程設計與執行結果 |
| [02-user-donor.md](02-user-donor.md) | 用戶・捐助者（訪客瀏覽、登入認捐、眾籌、儲值、退款）流程設計與執行結果 |
| [03-org.md](03-org.md) | 公益／慈善組織（申請、審核往返、成員、救災事件與據點、實收）流程設計與執行結果 |
| [04-operator.md](04-operator.md) | 平台營運者（組織審核、送禮執行、內容治理、客服、金流營運、品牌、回饋）流程設計與執行結果 |
| [findings.md](findings.md) | 彙整：發現的問題、已修正項目、開立的 issue、阻擋與目前缺失 |

## 狀態標記

| 標記 | 意義 |
|---|---|
| ✅ pass | 依設計走完，結果符合預期 |
| ❌ fail | 產品行為錯誤（bug） |
| ⚠️ gap | 能走但體驗或功能缺失（使用者會卡住、需靠繞道、文件與行為不一致） |
| ⛔ blocked | 本環境無法執行（外部服務、真機、金鑰），已記錄原因 |

## 模擬環境

所有服務在本機（雲端容器）啟動；腳本與共用工具在 `apps/web/qa/role-flows/`。

| 服務 | 位址 | 說明 |
|---|---|---|
| PostgreSQL 16 + PostGIS | `localhost:5432`，資料庫 `wishly_flow` | `docker compose up -d db` |
| Mailpit | SMTP `:1025`、Web/API `:8025` | Docker Hub 回 429（匿名拉取限流），改用 Mailpit 官方 release 執行檔 |
| S3 相容儲存 | `:9000`，bucket `wishly-media` | MinIO 映像同樣拉不到、`dl.min.io` 被出口代理封鎖，改用 `moto_server`（Python）；**不支援 R2 的條件式 PUT 語意保證**，圖片上傳結果僅供參考 |
| API | `:8080` | `APP_ENV=dev`、`APP_URL=http://localhost:3000`、`SMTP_PORT=1025`、`ECPAY_QUERY_URL=http://127.0.0.1:8090/query`、`ORGS_ENABLED` 走 dev 預設（開） |
| Web | `:3000` | 正式建置（node-server preset，即 `e2e/.app` layer）以 `NUXT_PUBLIC_API_BASE=http://localhost:8080` 建置後 `node .output/server/index.mjs` |
| 綠界替身 | `:8090` | `node qa/role-flows/ecpay-sim.mjs`：QueryTradeInfo 替身；付款成功由 `ecpayPaid()` 送簽章通知模擬 |

啟動順序（重現用）：

```sh
docker compose up -d db
createdb -h localhost -U wishly wishly_flow                # 密碼 wishly
./mailpit --smtp 0.0.0.0:1025 --listen 0.0.0.0:8025 &      # 或 docker compose up -d mailpit
moto_server -H 0.0.0.0 -p 9000 & curl -X PUT localhost:9000/wishly-media   # 或 docker compose up -d minio
node apps/web/qa/role-flows/ecpay-sim.mjs &
APP_ENV=dev DATABASE_URL=postgres://wishly:wishly@localhost:5432/wishly_flow APP_URL=http://localhost:3000 \
  SMTP_PORT=1025 ECPAY_QUERY_URL=http://127.0.0.1:8090/query cargo run --manifest-path apps/api/Cargo.toml &
(cd apps/web/e2e/.app && NUXT_PUBLIC_API_BASE=http://localhost:8080 ../../node_modules/.bin/nuxt build && PORT=3000 node .output/server/index.mjs &)
cd apps/web && node qa/role-flows/<role>.mjs              # 結果：qa/out/role-flows/<role>.json 與截圖（不進 git）
```

## 模擬原則

- **能用 UI 就用 UI**：每一步盡量以真實點擊、輸入完成；只有「平台外」的事（收信取 OTP、把帳號設為 staff、時間快轉、金流付款）才走 Mailpit / SQL / 替身，並在步驟中註明。
- 每個「人」一個獨立瀏覽器情境（cookie 隔離），桌面 1280 寬與手機 390 寬視流程選用。
- 外部依賴（Google/LINE 登入、Turnstile、真實綠界、Cloudflare 快取清除、真機 LINE 內建瀏覽器）無法在此環境驗證，列為 ⛔ 並寫明原因。
