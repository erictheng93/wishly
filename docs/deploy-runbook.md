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
   curl -s -o /dev/null -w "%{http_code}\n" https://api.wishly.tw/readyz    # 200
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
- 高風險 migration 前先手動備份並記錄。目前尚無自動備份（見 `docs/TODO.md` P0），別假設可以還原。

## 4. 回滾

- **後端**：Dokploy 保留前一個部署版本，可一鍵切回。確認新 migration 與舊版相容後才回滾。
- **前端**：Cloudflare Pages 部署紀錄切回前一版。
- **資料**：只能靠備份，見 §3。

## 5. 更新前後速查

- [ ] CI `publish-image` 綠色，`latest` SHA 正確
- [ ] 有 migration？向前相容、舊檔未改
- [ ] Dokploy `api` 的 Provider 是 Docker，部署後映像有 GHCR 的 RepoDigest（`docker image inspect`）
- [ ] Dokploy 只重新部署 `api`
- [ ] `/healthz`、`/readyz` 皆 200
- [ ] 前端首頁 200，手動走一次捐助流程
