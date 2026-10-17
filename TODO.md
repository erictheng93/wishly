# 待辦

## P0 上線阻擋

- [ ] Spike S1–S3 真機／真實鏈路驗收（無結案證據，補測優先於新功能）— 門檻與結果紀錄見 [docs/spike-results.md](docs/spike-results.md)（門檻待產品負責人確認後凍結；後端與前端已部署）
  - [ ] 前置（需產品負責人）：在正式環境建測試清單——① 一份 public、品項數量很大（如 needed=999）→ 取 slug 與品項 UUID（S3 觸發事件用）；② 一份「長中文標題＋封面圖」的 public 清單 slug；③ 一份 friends 或 password 清單（S1 受限預覽檢查）。S3 探針會對測試清單建立真實捐助（約 120 筆／120 分鐘），只能用測試清單。
  - [ ] S1 OG 預覽：爬蟲 UA curl 檢查（Claude 可跑）＋ LINE／FB／iMessage 真機貼連結截圖（需人工），6 組全過。
  - [ ] S2 LINE 內建瀏覽器身分存活：≥4 台真機（iOS×2、Android×2）× 5 輪 × 2 情境，存活率 ≥95%（需人工；Claude 無法執行）。
  - [ ] S3 SSE：`scripts/sse-probe.mjs` 跑 120 分鐘（有線＋手機 tethering 各一）；先自 Dokploy 取 Traefik 逾時與緩衝設定、cloudflared 版本；另測背景 5 分鐘與飛航模式降級（人工）。
  - [ ] 結論寫入 docs/spike-results.md 總表，證據放 `docs/spike-evidence/`；三項皆「通過」或「失敗＋備案已驗證」才解除 P0。

- [ ] 正式環境尚未部署（2026-10-15 嘗試部署 Cloudflare Pages 時發現，故未執行）
  - 後端（Linode VPS／Dokploy，D49）尚無部署證據，`https://api.wishly.tw` 不確定可連；同時確認 Traefik／Tunnel 的逾時不會截斷 SSE。
  - 取得 Turnstile 網站金鑰（`NUXT_PUBLIC_TURNSTILE_SITE_KEY`，前端建置必填）與後端 `TURNSTILE_SECRET`。
  - 定案正式網域（見 `docs/domain-names.md`）；Pages 專案 `wishly-web` 已存在（2026-10-16 已刪除重建，pages.dev 子網域為 wishly-web-5pd；網域 wishly.tw 待重新綁定）。
  - 後端正式機密、`TRUSTED_PROXY=cloudflare`（API 只能經 Tunnel 到達）、S3／R2、`SHIPPING_ENC_KEY` 備份流程（遺失即無法解密收件資訊）。
- [ ] 後端可部署性（部署定案：Linode VPS 東京 + Dokploy + Cloudflare Tunnel，PRD D49；清單見 [deploy-backend-checklist](docs/deploy-backend-checklist.md)）
  - [x] Dockerfile、CI build job、`/healthz`、`/readyz`（SIGTERM 優雅關閉曾實作，後因不需要而移除；換版時進行中請求會被切斷，可由 `Idempotency-Key` 重試恢復）。
  - [x] 背景工作排程：VPS 行程長駐，維持行程內排程，不實作 `/internal/jobs/tick`（原 Cloud Run 疑慮不成立）。
  - [ ] 背景任務 panic 會讓該任務靜默停止而行程仍存活：外層捕捉並重啟，或結束行程由 Dokploy 重啟；並加「逾期捐助未釋放」告警（checklist §0.6）。
  - [ ] **PostgreSQL 備份與還原**：WAL 歸檔或排程 dump 二選一並由產品負責人接受對應 RPO；備份至 R2；完成一次還原演練並記錄 RTO（checklist §2.1）。沒有可還原的備份不得開放真實使用者。
  - [ ] VPS 加固：入站全拒絕（Tunnel 為出站）、SSH 限制、Dokploy 管理介面不公開、日誌輪替、安全更新（checklist §2.3）。
  - [ ] 實機確認 Dokploy／Traefik／Tunnel 行為：備份機制、部署啟停順序、SSE 不被緩衝、`CF-Connecting-IP` 傳遞（checklist §10，與 Spike S3 同做）。
  - [ ] 多實例（replicas>1）前先確認各背景工作冪等（checklist §0.4）；在此之前維持 1 個。
  - [ ] `SHIPPING_ENC_KEY` 備份與資料庫備份分開保管（同機後威脅模型變弱）。
  - [ ] R2 物件儲存（已決）：建立 bucket（location hint、獨立 staging／production）、bucket 限定 API Token、自訂網域公開讀取、CORS；**`incoming/` 以 WAF 封鎖並設 1 天生命週期**（內含未去 EXIF 的原圖）；以真實 R2 驗證 presigned PUT（簽入 content-length）（checklist §2.2）。

## 功能與後續

- [x] 認捐（心願眾籌）逐筆可見度設定 — [#3](https://github.com/erictheng93/wishly/issues/3)
  - 已完成：migration 0006 回填並保留原有欄位；建立時套帳號預設或指定 visibility，逐筆 PATCH；個人頁、錢包與資料匯出支援逐筆可見度；轉投保留設定，匿名認捐在個人頁僅本人可見。

- [x] FundSheet 增加認捐當下的「誰看得到」欄位（issue #3 後續）：預設帶帳號 default_claim_visibility，可在送出前調整。

- [ ] 上線前以真實憑證驗證 Google／LINE 登入與 S3 頭像轉存 — [#6](https://github.com/erictheng93/wishly/issues/6)

- [ ] 頭像轉存後續（登入方式管理 ac8da6d）
  - 背景轉存每次登入都 spawn 一個任務，沒有並行上限；加 semaphore。
  - 綁定成功導回 /settings 時頭像可能尚未就緒，需重新整理才會出現。

- [ ] 安全與維運後續（登入方式管理）
  - link OTP 與登入 OTP 共用每 email 每小時 5 封額度：已登入者可對任一信箱耗盡額度，讓對方該小時無法用 OTP 登入。
  - 綁定流程沒有重新驗證身分（re-auth），需決定是否要求。
  - `delete_me` 取全域 advisory lock（staff 保護），刪帳號頻率低時無礙，量大再改。

- [ ] 原生殼（Tauri）上線前實機驗證：以下我只在無畫面環境驗證到 API／編譯層，需在真實視窗走一遍
  - 打包版 `.app`（`tauri dev` 在 macOS 收不到 deep link）：Google／LINE 登入回到 App、重啟仍登入、登出後重啟回登入頁。
  - 設定頁綁定 Google／LINE（含已屬於另一帳號的錯誤訊息）。
  - 編輯頁「複製連結」、「用 LINE 傳」、`sms:` 經 opener 開啟，確認 capability scope 放行。
  - Android：`npm run tauri:android-init` 實際接上 `tauri android init` 並確認 `allowBackup="false"`；登入、重啟保持登入、登出清除。
  - 後端 `NATIVE_ORIGINS`、`NATIVE_REDIRECT` 與供應商憑證（可與 #6 一併驗）。

- [ ] 原生殼：Universal Link／App Link（取代 `wishly://` 自訂 scheme）
  - 自訂 scheme 可被別的 App 搶註冊；目前靠 PKCE 讓攔截到的碼無法兌換，完整防護仍需 `apple-app-site-association`、`assetlinks.json`。
  - 需要正式網域（見 `docs/domain-names.md`）；同時可處理分享連結直接開 App。

- [ ] 原生殼：Android 權杖改用 Keystore 加密
  - 目前存 App 私有檔（`src-tauri/src/file_store.rs`，0600），僅靠沙箱；root 或允許備份時可被取出。
  - 做法：Kotlin plugin（AES-GCM 金鑰存 AndroidKeyStore）；需要 Android SDK 才能編譯驗證，故未做。

- [ ] 原生殼其他後續
  - 權杖仍在 webview 記憶體，XSS 可讀（keychain 只解決靜態儲存）；評估縮短權杖壽命或綁裝置。
  - OAuth 進行中若 OS 回收 App，verifier（只在記憶體）遺失需重新登入。
  - SSE（EventSource 無法帶 header）在原生殼退化為輪詢；可改 fetch streaming。
  - manifest 補 `screenshots`；iOS／Android 實際建置與上架流程。
  - 舊的 `X-Native-Client` 簽發權杖若存在仍有效（原生殼尚未發行，發行前可忽略）。

- [ ] 清單自動結束（`closes_at`）後續
  - 把「有效狀態」收斂為單一來源（view 或 `effective_status()`），API 回應與 SQL 守衛共用，避免各路徑各寫一份 `closes_at > now()`。
  - 背景 job 的 tick 間隔與到期精度寫進文件（目前沒有明確承諾）。
