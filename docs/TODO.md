# 待辦（上線與營運）

優先級：**P0** 資料可能無法恢復，先做；**P1** 安全／可用性，上線前完成；**P2** 驗收與品質；**P3** 整理。
做完請把條目移到最下方「已完成」並附日期與證據。背景與證據見 `docs/deploy-verification-2026-10-17.md`，上線條件見 `docs/deploy-backend-checklist.md`。

---

## P0 — 資料庫備份與還原演練

**為什麼是第一優先**：正式環境已有真實資料寫入，PostgreSQL 與後端同機（單點），目前**沒有任何備份**。`SHIPPING_ENC_KEY` 又沒有輪替機制，資料與金鑰任一遺失都無法恢復。

- [x] **決策**（2026-10-21，產品負責人）：兩者都做。方案 B（每小時 `pg_dump`；2026-10-27 曾改為 8 小時，評估容量後改回每小時，30 天約 100 MB），再做方案 A（WAL-G → R2）以達成 PRD 的 RPO ≤ 5 分鐘、PITR 7 天。操作見 `deploy-runbook.md` §6。
- [x] 方案 B 上線（2026-10-21）：`scripts/db-backup.sh` 部署到 VPS `/opt/wishly-backup/`，`/etc/cron.d/wishly-backup` 每小時執行；`-Fc -Z zstd` 先壓縮再存與傳，本機保留 48 份（0600）。沒有使用 Dokploy 內建備份：它只支援 Dokploy 的 Database 類型，`db` 是 Compose 服務。
- [x] R2 bucket `wishly-db-backups`（2026-10-27，APAC）：`postgres/` 設 Bucket Lock 30 天（VPS 被入侵、金鑰外流也刪不掉舊備份），生命週期第 31 天刪除（晚於鎖定，避免衝突）。
- [x] R2 備份憑證（2026-10-27）：專用 token 只限 `wishly-db-backups`、IP 限 VPS 的 IPv4，寫在 VPS `/etc/wishly-backup/r2.env`（0600）。VPS 有動態 IPv6，rclone 以 `--bind 0.0.0.0` 強制走 IPv4，否則會被 IP 過濾擋下（403）。實測以 token 刪除備份得到 `409 ObjectLockedByBucketPolicy`。
- [ ] 備份與 `SHIPPING_ENC_KEY` **不放在同一處**；確認該金鑰已存放於**兩個獨立位置**（密碼管理器＋離線）。
- [x] **還原演練（本機副本）**（2026-10-21）：`db-restore-drill.sh` 在 VPS 上把 `wishly-20261021T090717Z.dump` 還原到無網路的暫存容器，migration 28 全部成功，對帳通過（12 users／8 wishlists／16 items／2 claims），**資料庫還原耗時 8 秒**。之後每季一次。
- [x] 還原演練（從 R2 下載）（2026-10-27）：`wishly-20261027T085138Z.dump`（153 KB）從 R2 下載，與本機副本位元組相同；還原後對帳通過（12 users／9 wishlists／19 items／3 claims），耗時 7 秒。
- [ ] 備份新鮮度告警：註冊 healthchecks.io（或同類服務，週期 1 小時、寬限 30 分；WAL 歸檔失敗也會讓它不 ping），把 ping 網址填進 `r2.env` 的 `PING_URL`。腳本只有全部成功才會 ping。
- [x] 方案 A：WAL-G v3.0.9 上線（2026-10-27）：`archive_mode=on`、`archive_timeout=300`（RPO ≤ 5 分鐘），每日基底備份保留 8 份（PITR 7 天），R2 `wal-g/`。`db` 重建一次，停機約 10 秒，`/readyz` 200。PITR 演練：取回基底備份、重播 2 段 WAL、升格後對帳通過，耗時 9 秒。
- [x] Dokploy 後台的 `db` compose 已補上 `- /opt/wishly-backup/walg:/walg:ro`（2026-10-27，已比對 Dokploy 存的 `composeFile`、VPS 上的檔案與執行中容器的掛載，三者一致）。
- [x] 指定時間點的 PITR 演練（2026-10-27）：正式庫暫建 `ops_drill.marker`，09:17:48 寫 A、09:17:53 寫 B；`walg-restore-drill.sh '2026-10-27 09:17:50.892224+00'` 還原後只有 A、沒有 B，對帳通過，耗時 9 秒；演練後已 drop 該 schema。
- [x] 整機恢復手冊（2026-10-27）：`deploy-runbook.md` §8；WAL-G 安裝改為可重跑的 `scripts/walg-setup.sh`（在正式機重跑，產出與現行檔案 sha256 相同）。
- [ ] 依 runbook §8 實際演練並計時（需開一台暫時的 Linode，演練完刪除；對外的 Tunnel／DNS 不切換）。
- [ ] Linode 整機備份／快照啟用，視為輔助（不等於資料庫一致性備份）。
- [x] 高風險 migration 前先手動備份：已寫入 runbook §3（執行 `db-backup.sh`）。

相關：清單 §2.1、§7.13、§8。

---

## P1 — 安全與可用性（上線前）

- [ ] **輪替驗證過程中外露的機密**（見驗證紀錄 §12.1）：`DATABASE_URL` 的 DB 密碼（用 `ALTER ROLE wishly PASSWORD …`，不必重建 `db` 服務，並同步更新 `db` 服務與 `api` 的環境變數）、`OTP_PEPPER`、`OAUTH_SECRET`、`UNSUB_SECRET`、`TURNSTILE_SECRET`（Cloudflare 上 Roll）。**`SHIPPING_ENC_KEY` 在有真實收件資訊之前輪替最便宜，之後就不能換了**，請先決定。新值只放密碼管理器與 Dokploy，不貼給任何人或代理程式。
- [ ] **機密外洩防線（本 repo 為公開）**：GitHub secret scanning 與 push protection 已開（2026-10-17 以 `gh api` 確認），但它們只認已知服務商的 token 格式，**認不出十六進位密鑰、`postgres://` 連線字串、Turnstile secret**。補上通用掃描：CI 加 `gitleaks`（或 `trufflehog`）掃 PR 與 push，並建議本機 pre-commit 掃 staged 檔。同時固定提交習慣：只用 `git add <指定檔案>`，不用 `git add .`／`-A`。
- [ ] **瀏覽器自動化工具的輸出不得含機密**：操作含機密的頁面（Dokploy 環境變數、Cloudflare Token 頁、GitHub PAT 頁）時，不存快照，只用 `eval` 取鍵名／行數；工具的暫存目錄（`.playwright-cli/`）已加進 `.gitignore`，但這只防誤提交，不防機密進入對話上下文。
- [ ] **`/readyz` 對外封鎖**（它查 DB，目前公網可達並回 200）：Cloudflare WAF 規則只放行帶 `X-Monitor-Key` 的請求，做法見 `docs/deploy-runbook.md`「外部監控」。`/healthz` 可開放。
- [ ] Dokploy 管理員啟用 **2FA**；確認管理介面（3000）只透過 SSH 通道進入。
- [ ] **SSH 22 加固**：限來源 IP、僅金鑰登入、關閉密碼登入、fail2ban。注意**不要動外層防火牆既有規則**（Docker 已發布 80/443/3000，靠它擋住）。
- [ ] 確認 80／443／3000 不可從公網直連（目前只驗證了 8080，見驗證紀錄 §5.3）。
- [ ] 外部監控與告警：~~打 `/readyz` 的外部探測~~（`.github/workflows/uptime.yml`，每 5 分鐘，WAF 生效後才算完成）；5xx 比率、p95 延遲、磁碟（含 WAL／備份）、記憶體、容器重啟次數、Tunnel 狀態、寄信佇列積壓、逾期捐助未釋放數量。
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
- [ ] 預設封面分享預覽（PR #36）：部署後拿正式網址貼到 LINE，確認預覽圖會顯示預設封面（`/covers/{id}.png`），而非 `og-default.png`。
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
- [ ] `GET /friends/feed`（好友動態，`friends.rs::feed`）尚未寫進 `docs/04` API 契約。

## 好友動態後續（2026-10-20 方案 A 已上 `/friends`，提案稿已刪）

- [ ] 眾籌「快達標」卡（例：達 80% 且未截止），需在 feed 加 crowdfund 品項的進度事件。
- [ ] 共同好友數、「好友 N 年」（`friendships.created_at` 已有）、常看好友捷徑。
- [ ] 「提醒他建清單」：生日近但沒有可見清單的好友，送通知（需防騷擾限流）。
- [ ] 已知限制：動態以品項 `created_at` 計，草稿放超過 30 天才發佈的清單不會出現；考慮加 `wishlists.published_at`。
- [ ] 另一方向（提案 B 倒數看板）：以日期為主角，大數字倒數＋60 天時間軸（生日、`event_date`、`closes_at` 混排），可做成 `/friends` 的另一個分頁。

---

## 已完成

- 2026-10-17 GHCR 映像由 CI 推送、Dokploy 部署、Tunnel 公開 `api.wishly.tw`（驗證紀錄 §3–§5）
- 2026-10-17 前端 `wishly.tw` 綁定 Pages（§6）
- 2026-10-17 R2 bucket／CORS／Lifecycle／自訂網域／WAF 封鎖 `incoming/`（§7）
- 2026-10-17 Email Service 寄件網域與 SPF/DKIM、OTP 信實收（§8）
- 2026-10-17 註冊、建清單、圖片上傳、發布（§9）
- 2026-10-17 公開頁 Cache Rule 與 Cache Purge 實測（§10）
- 2026-10-17 SSE 3 分鐘短測（§11；完整驗收仍在 P2）
