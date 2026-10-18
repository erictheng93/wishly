# 待辦（上線與營運）

優先級：**P0** 資料可能無法恢復，先做；**P1** 安全／可用性，上線前完成；**P2** 驗收與品質；**P3** 整理。
做完請把條目移到最下方「已完成」並附日期與證據。背景與證據見 `docs/deploy-verification-2026-10-17.md`，上線條件見 `docs/deploy-backend-checklist.md`。

---

## P0 — 資料庫備份與還原演練

**為什麼是第一優先**：正式環境已有真實資料寫入，PostgreSQL 與後端同機（單點），目前**沒有任何備份**。`SHIPPING_ENC_KEY` 又沒有輪替機制，資料與金鑰任一遺失都無法恢復。

- [ ] **決策並寫入文件**：方案 A（WAL 歸檔＋基底備份，pgBackRest／WAL-G → R2，RPO 約數分鐘、可 PITR）或方案 B（排程 `pg_dump`，RPO = 備份間隔，PRD 的 RPO ≤ 5 分鐘需改寫並由產品負責人接受）。
- [ ] 先**實測 Dokploy 內建備份**：實際機制（是否 `pg_dump`、能否 PITR、最短排程間隔）、是否支援 Compose 型服務（`db` 是 Compose，不是 Dokploy 管理的 Database 類型，可能不適用）。
- [ ] 備份目的地用 **R2 的獨立 bucket**（例如 `wishly-backup`），使用**另一組** S3 API Token（只限該 bucket；不得重用 `wishly-media` 的金鑰，也不可放在 VPS 的應用環境變數裡讓應用能讀）。
- [ ] 備份與 `SHIPPING_ENC_KEY` **不放在同一處**；確認該金鑰已存放於**兩個獨立位置**（密碼管理器＋離線）。
- [ ] **還原演練**：還原到暫存實例（不碰正式 `db` 服務），啟動後端對帳（同 `assert_all_reconciled` 的檢查）通過，記錄實際 **RTO**；之後每季一次。
- [ ] 備份新鮮度告警：備份失敗或超過 N 小時未成功要通知（備份靜默失敗是最常見的事故）。
- [ ] 寫「整機掛掉」恢復手冊並計時：新 VPS → Dokploy → 還原 DB → 設環境變數 → Tunnel。
- [ ] Linode 整機備份／快照啟用，視為輔助（不等於資料庫一致性備份）。
- [ ] 高風險 migration 前先手動做一次備份並記錄（回滾只能靠備份）。

相關：清單 §2.1、§7.13、§8。

---

## P1 — 安全與可用性（上線前）

- [ ] **輪替驗證過程中外露的機密**（見驗證紀錄 §12.1）：`DATABASE_URL` 的 DB 密碼（用 `ALTER ROLE wishly PASSWORD …`，不必重建 `db` 服務，並同步更新 `db` 服務與 `api` 的環境變數）、`OTP_PEPPER`、`OAUTH_SECRET`、`UNSUB_SECRET`、`TURNSTILE_SECRET`（Cloudflare 上 Roll）。**`SHIPPING_ENC_KEY` 在有真實收件資訊之前輪替最便宜，之後就不能換了**，請先決定。新值只放密碼管理器與 Dokploy，不貼給任何人或代理程式。
- [ ] **機密外洩防線（本 repo 為公開）**：GitHub secret scanning 與 push protection 已開（2026-10-17 以 `gh api` 確認），但它們只認已知服務商的 token 格式，**認不出十六進位密鑰、`postgres://` 連線字串、Turnstile secret**。補上通用掃描：CI 加 `gitleaks`（或 `trufflehog`）掃 PR 與 push，並建議本機 pre-commit 掃 staged 檔。同時固定提交習慣：只用 `git add <指定檔案>`，不用 `git add .`／`-A`。
- [ ] **瀏覽器自動化工具的輸出不得含機密**：操作含機密的頁面（Dokploy 環境變數、Cloudflare Token 頁、GitHub PAT 頁）時，不存快照，只用 `eval` 取鍵名／行數；工具的暫存目錄（`.playwright-cli/`）已加進 `.gitignore`，但這只防誤提交，不防機密進入對話上下文。
- [ ] **`/readyz` 對外限流或封鎖**（它查 DB，目前公網可達並回 200）。`/healthz` 可開放。外部監控改走允許來源。
- [ ] Dokploy 管理員啟用 **2FA**；確認管理介面（3000）只透過 SSH 通道進入。
- [ ] **SSH 22 加固**：限來源 IP、僅金鑰登入、關閉密碼登入、fail2ban。注意**不要動外層防火牆既有規則**（Docker 已發布 80/443/3000，靠它擋住）。
- [ ] 確認 80／443／3000 不可從公網直連（目前只驗證了 8080，見驗證紀錄 §5.3）。
- [ ] 外部監控與告警：打 `/readyz` 的外部探測；5xx 比率、p95 延遲、磁碟（含 WAL／備份）、記憶體、容器重啟次數、Tunnel 狀態、寄信佇列積壓、逾期捐助未釋放數量。
- [ ] `cloudflared` 容器確認 restart policy 為 always；Tunnel 掉線告警（Tunnel 是單點，掛掉全站不可達）。
- [ ] 記錄並設定提醒：GHCR 讀取 PAT 的**到期日**；R2 S3 Token 若有到期日也一併記錄（到期會讓重新部署／上傳突然失敗）。
- [ ] VPS 記憶體實測：`free -h`（DB、Dokploy、cloudflared、後端同機執行）。
- [ ] **每月檢查一次 braces／node-forge 兩個公告是否有修補版**（CI `web-audit` 已移除；issue #7 已關閉，需要時重開）：`gh api /advisories/GHSA-vfj7-8cjw-p6xm` 與 `/advisories/GHSA-86w9-cpqp-85rv` 看 `first_patched_version`，並 `npm view braces version`、`npm view node-forge version`。有修補版後依 `apps/web/patches/README.md` 更新 lockfile、重跑驗證、移除暫時補丁並關閉 #7。上次檢查：2026-10-17（兩者皆無修補版）。

---

## P2 — 驗收與品質

驗收（清單 §7，**未驗證**的項目）：
- [ ] **SSE 完整驗收**：`DURATION_MIN` ≥ 40 確認越過 30 分鐘上限會重連；Spike S3 為 120 分鐘，門檻 0 次 > 30 s 間隔、事件延遲 p50≤2 s／p95≤5 s／max≤10 s。**注意 `events_missed` 在單一品項 `quantity=1` 下會誤報**（後續觸發回 409 不產生事件），測試前換成數量足夠的品項。
- [ ] §7.3 訪客捐助與防超賣（staging 走一遍，不要在正式環境跑大量寫入）。
- [ ] §7.4 限流以真實用戶 IP 計（兩個不同 IP 互不影響；驗證 `TRUSTED_PROXY` 與 `CF-Connecting-IP`）。
- [ ] §7.6 `scripts/cdn-canary.mjs` exit 0。
- [ ] §7.9 檢舉＋Turnstile：偽造 token 得 403。
- [ ] §7.11 背景工作：建一筆短效期捐助，確認到期自動釋放。
- [ ] §7.12 Google／LINE OAuth 各登入一次（issue #6）。
- [ ] §7.14 重新部署期間進行中請求的失敗數與捐助一致性。
- [ ] §7.15 手動 kill 容器，確認自動重啟、Tunnel 恢復。
- [ ] 延遲：從**其他網路／機器**重測 `api.wishly.tw`（本次從單一網路量到 connect 最高 5 秒、`cf-ray` 為 SJC，判斷是本機網路抖動但無法定論）。
- [ ] 寄信：用 Gmail 以外的信箱測 OTP；確認不進垃圾信。穩定後再決定 DMARC 是否維持 `p=reject`（觀察期先 `p=none`；注意 SPF 因 `cf-bounce` 子網域在嚴格對齊下不對齊，DMARC 靠 DKIM 通過）。

CI／部署流程：
- [ ] 評估 `ubuntu-latest` 於 **2026-10-19** 遷至 Ubuntu 26 的影響（`api`／`e2e`／`image`）；必要時固定 `ubuntu-24.04`。
- [ ] **自動部署**：CI `publish-image` 完成後呼叫 Dokploy webhook 以該次 SHA 部署，或關閉 Dokploy 的 `Autodeploy`（目前開啟但無作用）。
- [ ] 回滾演練：把 `api` 的 Image 標籤改回前一個 SHA 並部署。

---

## P3 — 整理與一致性

- [ ] 刪除或保留驗證用測試資料（帳號、清單「部署驗證用清單」、品項、捐助、R2 封面圖）。
- [ ] Problem `type` URI 統一：後端 `https://wishly.tw/problems/…` vs `docs/04` 的 `https://api.wishly.tw/problems/…`。
- [ ] 本機開發環境：`apps/api/.env` 仍是舊網域；本機 DB 需 `docker compose down -v && docker compose up -d` 才套用新帳號（會清掉本機資料，由使用者決定）。
- [ ] Tauri 原生殼：識別碼 `tw.wishly.app`、scheme `wishly://` 已更名，尚未編譯驗證。
- [ ] `www.wishly.tw` 是否需要導向 `wishly.tw`。
- [ ] 帳號權限最小化：帳號與其他專案共用（`makanmasak.com`、`prophetica.net`、多個 R2 bucket、一個 All-buckets 的 R2 Admin Token）；日後新增 Cloudflare 資源時注意 Domain／bucket 預設值會指到別的專案。
- [ ] Dokploy 設定備份（專案、環境變數、Tunnel token 的存放位置）。
- [ ] Dokploy Settings → Registry 目前沒被使用（Docker Provider 用自己的帳密欄），可移除以免誤導。

---

## 已完成

- 2026-10-17 GHCR 映像由 CI 推送、Dokploy 部署、Tunnel 公開 `api.wishly.tw`（驗證紀錄 §3–§5）
- 2026-10-17 前端 `wishly.tw` 綁定 Pages（§6）
- 2026-10-17 R2 bucket／CORS／Lifecycle／自訂網域／WAF 封鎖 `incoming/`（§7）
- 2026-10-17 Email Service 寄件網域與 SPF/DKIM、OTP 信實收（§8）
- 2026-10-17 註冊、建清單、圖片上傳、發布（§9）
- 2026-10-17 公開頁 Cache Rule 與 Cache Purge 實測（§10）
- 2026-10-17 SSE 3 分鐘短測（§11；完整驗收仍在 P2）
