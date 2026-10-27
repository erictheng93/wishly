# 部署手冊（日常更新）

首次部署與完整檢查清單見 `deploy-backend-checklist.md`、`deploy-verification-2026-10-17.md`。本頁只寫「之後每次更新」的步驟。

**原則：永遠不在 VPS 編譯。** 後端映像只由 GitHub Actions 建置並推到 GHCR，VPS 上的 Dokploy 只拉映像並重新部署。

| 部分 | 位置 | 怎麼更新 |
|---|---|---|
| 前端 | Cloudflare Pages（`wishly-web`，`https://wishly.tw`） | 本機執行 `npm run deploy`（§2） |
| 後端 | Linode 東京 VPS，Dokploy 管理，經 Cloudflare Tunnel（`https://api.wishly.tw`） | CI 推映像 → 手動在 Dokploy 重新部署（§1） |
| 資料庫 | 同一台 VPS，Dokploy 的 `db` 服務 | 隨後端啟動自動套用 `db/migrations`（§3） |

## 1. 後端

1. **推到 main，等 CI。** `git push` 後，到 `https://github.com/erictheng93/wishly/actions` 看最新一次 `ci`。
   - `publish-image` 必須是綠色（它需要 `api`、`image`、`web`、`e2e` 全綠）。
2. **確認映像版本。** 到 `https://github.com/erictheng93?tab=packages` → `wishly-api`，確認 `latest` 旁的 commit SHA 就是你要部署的 commit（與 `git log` 對照）。映像是 Private，需登入 GitHub。
3. **開隧道進 Dokploy。** 管理介面不對外：
   ```sh
   ssh -L 3001:localhost:3000 <VPS>
   ```
   瀏覽器開 `http://localhost:3001`。
4. **重新部署 `api`。** 專案 `wishly` → 環境 `production` → Application `api`（App Name `wishly-api-qyob9a`）→ Deploy / Redeploy，讓它重新拉 `ghcr.io/erictheng93/wishly-api:latest`。
   - **不要刪除重建 `db` 服務**：DB 容器名會改變，`DATABASE_URL` 失效。
   - replicas 維持 1。
   - **General → Provider 必須是 Docker**（`ghcr.io/erictheng93/wishly-api:latest`，憑證填在該分頁）。若是 Git / Github，Dokploy 會在 VPS 上編譯（2026-10-21 曾被改成 Git）。Docker 模式下 Redeploy 只拉映像，幾秒就完成。
   - 不要在 VPS 上 `git pull` 或編譯。
5. **看 Dokploy 日誌**，確認啟動、migration 沒有錯誤。
6. **驗證：**
   ```sh
   curl -s -o /dev/null -w "%{http_code}\n" https://api.wishly.tw/healthz   # 200
   curl -s -o /dev/null -w "%{http_code}\n" https://api.wishly.tw/readyz    # 403（對外由 WAF 封鎖，正常）
   gh workflow run uptime && sleep 5 && gh run watch "$(gh run list -w uptime -L 1 --json databaseId -q '.[0].databaseId')"   # 帶金鑰探測，應成功
   ```
   再手動走一次捐助流程。

**換版注意**
- 程式不處理 SIGTERM，換版時進行中的請求會被切斷；捐助有 `Idempotency-Key`，客戶端重試可恢復。
- Dokploy 是否「先起新容器再停舊容器」尚未實測，換版瞬間可能有兩個實例並存。
- 拉映像用的 GitHub PAT 有到期日，過期會 401（影響重新部署，不影響執行中的容器）。到期日記錄在 `docs/TODO.md`。
- Dokploy 的 Application → Docker Provider 分頁有自己的 Registry 憑證欄位，Settings 裡的 Registry 沒有被使用。
- Dokploy 環境變數編輯器可能未遮蔽機密值：不要用瀏覽器自動化或代理程式操作，也不要截圖貼出。

## 2. 前端

在 `apps/web` 執行（Turnstile Site key 是公開值，Secret key 不是）：

```sh
cd apps/web
NUXT_PUBLIC_TURNSTILE_SITE_KEY=0x4AAAAAAFYJ9opHhXnE6Dt_ npm run deploy
```

- 缺少該變數時 `scripts/check-prod-env.mjs` 會中止，這是刻意的。
- 建置時 `NUXT_DEMO` 為空，所以正式站沒有 `/s/demo` 等示範頁（回 404）。
- 輸出裡 `node:process` 的 `sideEffects` 警告無害，可忽略（Nitro 入口檔與 wrangler 的 unenv-preset 對不上）。若出現 `process is not defined` 或 `Buffer is not defined` 之類執行期錯誤才需處理。
- 驗證：`curl -s -o /dev/null -w "%{http_code}\n" https://wishly.tw/` 為 200，並開頁面看一次。

## 3. 有 migration 的版本

- Migration 只向前相容（expand → migrate → contract）：**同一版不要移除舊欄位**。回滾後端時，資料庫已套用的 migration 必須與舊版程式相容。
- **已套用的 migration 檔案內容不可修改**（連註解也不行），sqlx 會因 checksum 不符拒絕啟動。要改請新增 migration。
- 高風險 migration 前先手動執行一次 `/opt/wishly-backup/db-backup.sh`（§6），確認輸出 `ok` 再部署。

## 4. 回滾

- **後端**：Dokploy 保留前一個部署版本，可一鍵切回。確認新 migration 與舊版相容後才回滾。
- **前端**：Cloudflare Pages 部署紀錄切回前一版。
- **資料**：只能靠備份，見 §6。

## 5. 更新前後速查

- [ ] CI `publish-image` 綠色，`latest` SHA 正確
- [ ] 有 migration？向前相容、舊檔未改
- [ ] Dokploy `api` 的 Provider 是 Docker，部署後映像有 GHCR 的 RepoDigest（`docker image inspect`）
- [ ] Dokploy 只重新部署 `api`
- [ ] `/healthz` 200；`/readyz` 對外 403、`uptime` workflow 成功
- [ ] 前端首頁 200，手動走一次捐助流程

## 6. 資料庫備份與還原

兩套並行（2026-10-27 起）：**方案 A WAL-G**（WAL 持續歸檔＋每日基底備份，`archive_timeout=300`，**RPO ≤ 5 分鐘、PITR 7 天**）為主；**方案 B 每小時 `pg_dump`** 為獨立備援（有 Bucket Lock，VPS 被入侵也刪不掉）。腳本在 repo 的 `scripts/db-*`，VPS 上放在 `/opt/wishly-backup/`（改了腳本要用 `scp` 同步，不在 VPS 上 `git pull`）。

| 項目 | 位置 |
|---|---|
| 排程 | `/etc/cron.d/wishly-backup`，每小時第 17 分；日誌 `journalctl -t wishly-backup` |
| 本機副本 | `/var/backups/wishly/wishly-<UTC 時間>.dump`（0600，保留最近 48 份 ≈ 2 天） |
| 異地副本 | R2 `wishly-db-backups/postgres/`；憑證在 `/etc/wishly-backup/r2.env`（0600，範本 `scripts/db-backup.r2.env.example`）。檔案不存在時只做本機備份。`postgres/` 有 Bucket Lock 30 天（不可刪、不可覆寫），第 31 天由生命週期刪除 |
| 格式 | `pg_dump -Fc -Z zstd`：先壓縮再落地、上傳；`pg_restore` 直接讀 |
| WAL-G | 執行檔與設定在 `/opt/wishly-backup/walg/`（`walg.env` 0400、owner 999，由 `r2.env` 轉出），以 `/walg:ro` 掛進 `db`；歸檔設定用 `ALTER SYSTEM` 寫在資料目錄（`archive_mode=on`、`archive_command='/walg/archive.sh wal-push %p'`）。R2 前綴 `wishly-db-backups/wal-g/`，zstd 壓縮，**沒有 Bucket Lock**（WAL-G 要能自行清理） |
| 基底備份 | `/etc/cron.d/wishly-walg`，每日 03:47 UTC 執行 `walg-basebackup.sh`，保留 8 份，舊 WAL 一併刪除；日誌 `journalctl -t wishly-walg` |
| 歸檔監看 | `db-backup.sh` 每小時檢查 `pg_stat_archiver`，最近一次失敗晚於成功就以錯誤結束、不 ping |

⚠ **`db` 的 compose 必須含 `- /opt/wishly-backup/walg:/walg:ro`**（Dokploy 後台與 VPS 上的檔案都要有）。少了這行，容器重建後歸檔會一直失敗，WAL 堆積直到磁碟滿。

**還原演練**（每季一次，兩種都做，記錄在 `docs/TODO.md` 已完成區）：

PITR（方案 A）：`/opt/wishly-backup/walg-restore-drill.sh ['2026-10-27 09:00:00+00']`，從 R2 取最新基底備份、重播 WAL 到指定時間（省略則重播到最後），在暫存容器內升格後對帳；腳本會在暫存實例關掉 `archive_mode`，避免把新時間線推進正式歸檔。

邏輯備份（方案 B）：

```sh
ssh root@<VPS>
/opt/wishly-backup/db-restore-drill.sh "$(ls -1t /var/backups/wishly/*.dump | head -1)"
# 從 R2 演練：先 docker run --rm --env-file /etc/wishly-backup/r2.env -v /tmp:/data rclone/rclone:1.68.2 copy --bind 0.0.0.0 r2:wishly-db-backups/postgres/<檔名> /data/
```

腳本會在無網路的暫存容器還原，然後跑 `db-reconcile.sql`（與測試的 `assert_all_reconciled` 同條件）。最後一行 `還原演練通過…耗時 N 秒` 就是資料庫部分的 RTO，對帳失敗會以非 0 結束。

**真正還原到正式**（資料毀損、誤刪）：
1. Dokploy 停掉 `api`，避免繼續寫入。
2. 先備份現況：`/opt/wishly-backup/db-backup.sh`（留作事後分析）。
3. 在既有 `db` 容器內重建資料庫（**不要刪除重建 `db` 服務**，否則容器名改變）。postgis 映像在預設庫預裝了擴充，所以要從 `template0` 建，否則 `pg_restore` 會撞到 `schema "tiger" already exists`：
   ```sh
   D=wishly-db-2y7t3r-db-1
   docker exec $D psql -U wishly -d postgres -c "DROP DATABASE wishly WITH (FORCE)"
   docker exec $D createdb -U wishly -T template0 wishly
   docker exec -i $D pg_restore -U wishly -d wishly --no-owner --exit-on-error < /var/backups/wishly/<檔名>.dump
   docker exec -i $D psql -U wishly -d wishly -X < /opt/wishly-backup/db-reconcile.sql
   ```
4. 啟動 `api`，執行 §1 的驗證。

## 7. 外部監控（/readyz）

`.github/workflows/uptime.yml` 每 5 分鐘帶 `X-Monitor-Key` 打 `https://api.wishly.tw/readyz`，連續 3 次失敗就讓 workflow 失敗，GitHub 寄信告警。`/readyz` 會查 DB，所以對外用 WAF 封鎖，只放行帶金鑰的請求。Dokploy 的內部探針直連容器、不經 Cloudflare，不受影響。

**一次性設定（依序做；金鑰不要貼給任何人或代理程式）：**

1. 在自己的終端機產生金鑰，同時存進 GitHub secret 並複製到剪貼簿（不會印在螢幕上）：
   ```sh
   openssl rand -hex 32 | tr -d '\n' | tee >(pbcopy) | gh secret set MONITOR_KEY -R erictheng93/wishly
   ```
2. Cloudflare → wishly.tw → Security → WAF → Custom rules → Create rule，名稱 `block-public-readyz`，Edit expression 貼上（把 `<KEY>` 換成剪貼簿內容），Action 選 **Block**：
   ```
   (http.host eq "api.wishly.tw" and http.request.uri.path eq "/readyz" and not any(http.request.headers["x-monitor-key"][*] eq "<KEY>"))
   ```
3. 驗證：
   ```sh
   curl -s -o /dev/null -w "%{http_code}\n" https://api.wishly.tw/readyz   # 403
   gh workflow run uptime   # 到 Actions 頁確認成功
   ```
4. GitHub → Settings → Notifications → Actions：勾選 Email、「Only notify for failed workflows」。排程失敗的信寄給**最後修改排程的人**。

**收到告警時：**看失敗那次的訊息。403 = 金鑰與 WAF 規則不一致（換金鑰時兩邊要一起改）；503 = DB 連不到；000 / 52x = 伺服器或 Tunnel 掛了。

**限制：**GitHub 排程常延遲 5–20 分鐘；repo 60 天沒有任何活動，排程會被自動停用（GitHub 會先寄信）。需要更即時的告警時改用 Better Stack 等專門服務。
