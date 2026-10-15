# 待辦

## P0 上線阻擋

- [ ] Spike S1–S3 真機／真實鏈路驗收（無結案證據，補測優先於新功能）— 流程見 [spike-protocol](docs/qa/spike-protocol.md)，現況見 [spike-status](docs/qa/spike-status.md)，結果填 [spike-decisions](docs/qa/spike-decisions.md)
  - 先由產品負責人確認 S1–S3 門檻（測前定案，不可測後挑）。
  - S1 OG 預覽：6 組平台 × 連結；S2 LINE 內建瀏覽器身分存活：≥4 台裝置；S3 SSE：以 `scripts/sse-probe.mjs` 在 Cloudflare → Cloud Run 跑 120 分鐘。
  - 前置：S2／S3 需要真實後端與具名網域，見下一項。

- [ ] 正式環境尚未部署（2026-10-15 嘗試部署 Cloudflare Pages 時發現，故未執行）
  - 後端（Cloud Run 或具名通道）尚無部署證據，`https://api.wishsync.tw` 不確定可連；同時確認 Cloud Run request timeout（預設 300s）不會截斷 SSE。
  - 取得 Turnstile 網站金鑰（`NUXT_PUBLIC_TURNSTILE_SITE_KEY`，前端建置必填）與後端 `TURNSTILE_SECRET`。
  - 定案正式網域（見 `docs/domain-names.md`）；Pages 專案 `wishsync-web` 已存在，正式部署會覆蓋其目前內容（疑似 demo）。
  - 後端正式機密、`TRUSTED_PROXY`、S3／R2、`SHIPPING_ENC_KEY` 備份流程（遺失即無法解密收件資訊）。

## 功能與後續

- [x] 認捐（點數眾籌）逐筆可見度設定 — [#3](https://github.com/erictheng93/wishsync/issues/3)
  - 已完成：migration 0006 回填並保留原有欄位；建立時套帳號預設或指定 visibility，逐筆 PATCH；個人頁、錢包與資料匯出支援逐筆可見度；轉投保留設定，匿名認捐在個人頁僅本人可見。

- [ ] FundSheet 增加認捐當下的「誰看得到」欄位（issue #3 後續）：目前套帳號預設，認捐後可到錢包修改；預設公開時，修改前可能已被看到。

- [ ] 上線前以真實憑證驗證 Google／LINE 登入與 S3 頭像轉存 — [#6](https://github.com/erictheng93/wishsync/issues/6)

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

- [ ] 原生殼：Universal Link／App Link（取代 `wishsync://` 自訂 scheme）
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
