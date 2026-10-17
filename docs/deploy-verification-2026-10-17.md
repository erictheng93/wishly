# 首次部署與端對端驗證紀錄（2026-10-17）

本文件記錄 Wishly 後端首次部署到正式環境（Linode 東京 VPS + Dokploy + Cloudflare Tunnel）的過程、設定與驗證證據。
對照文件：`docs/deploy-backend-checklist.md`（上線條件清單）、`docs/TODO.md`（待辦與優先級）。

> **安全約定**：本文件不記錄任何機密值（密碼、token、secret、金鑰、DB 連線字串中的密碼）。只記錄鍵名、設定形狀與公開識別碼。
> 驗證者：Claude Code（代操作瀏覽器與指令）＋ 專案負責人（所有機密的輸入、登入、收信均由負責人親自操作）。

---

## 1. 結論摘要

| 範圍 | 狀態 | 證據位置 |
|---|---|---|
| 映像由 GitHub Actions 建置並推送 GHCR（私有） | 通過 | §3 |
| Dokploy 以 GHCR 映像部署後端，production 設定檢查通過 | 通過 | §4 |
| Cloudflare Tunnel 公開 `api.wishly.tw`，`/healthz`、`/readyz` 回 200 | 通過 | §5 |
| VPS 8080 不對外（後端只能經 Tunnel 存取） | 通過 | §5 |
| 前端 `wishly.tw` 綁定 Cloudflare Pages | 通過 | §6 |
| R2：bucket、CORS、Lifecycle、自訂網域、WAF 封鎖 `incoming/` | 通過 | §7 |
| Email Service：寄件網域、SPF/DKIM；OTP 信實際收到 | 通過 | §8 |
| 註冊／登入／建清單／發布 | 通過 | §9 |
| 圖片上傳（presign → R2 PUT → confirm → 去 EXIF → 公開網址） | 通過 | §9.2 |
| 公開頁 CDN 快取（Cache Rule） | 通過（原本缺，已補） | §10 |
| Cache Purge（`CF_ZONE_ID`、`CF_CACHE_PURGE_TOKEN`） | 通過 | §10.3 |
| SSE 經 Cloudflare → Tunnel（3 分鐘短測） | 通過（時間不足以驗收 Spike S3） | §11 |

**尚未完成且優先級最高：資料庫備份與還原演練**（見 `docs/TODO.md` P0）。目前正式環境已有真實資料寫入，但沒有任何備份。

另有一項**機密外露處理待辦**（§12.1）：驗證過程中 Dokploy 環境變數編輯器未遮蔽，值被讀入代理程式的對話紀錄。建議輪替（成本現在最低），見 `docs/TODO.md` P1。

---

## 2. 部署拓撲（實際）

```
瀏覽器 → Cloudflare（WAF／Cache Rules）─┬→ wishly.tw        → Cloudflare Pages（wishly-web）
                                        ├→ media.wishly.tw  → R2 bucket wishly-media（自訂網域）
                                        └→ api.wishly.tw    → Tunnel（cloudflared，出站）→ wishly-api-qyob9a:8080 → PostgreSQL（同機）
瀏覽器 ──直接 PUT（presigned）──→ <account>.r2.cloudflarestorage.com/wishly-media/incoming/...
```

| 元件 | 實際值 |
|---|---|
| VPS | Linode 東京，Ubuntu；對外僅 SSH 22 |
| Dokploy | v0.30.8；專案 `wishly` / 環境 `production`；管理介面不對外，用 `ssh -L 3001:localhost:3000` 進入 |
| Dokploy 服務 | `db`（Compose，`postgis/postgis:16-3.4`，volume `pgdata`）、`cloudflared`（Compose）、`api`（Application，App Name `wishly-api-qyob9a`） |
| 容器網路 | `dokploy-network`（overlay）；DB 容器名 `wishly-db-2y7t3r-db-1`（**不可刪除重建 `db` 服務，否則容器名改變、`DATABASE_URL` 失效**） |
| 後端映像 | `ghcr.io/erictheng93/wishly-api:<commit SHA>`（私有），平台 `linux/amd64` |
| Tunnel | 名稱 `wishly`，1 個 replica，狀態 Healthy；Published application：`api.wishly.tw` → `http://wishly-api-qyob9a:8080` |
| 前端 | Cloudflare Pages 專案 `wishly-web`（pages.dev 子網域 `wishly-web-5pd`），自訂網域 `wishly.tw` |
| 圖片 | R2 bucket `wishly-media`（apac），自訂網域 `media.wishly.tw`（最低 TLS 1.2） |

---

## 3. CI 與映像發布

### 3.1 新增 `publish-image` job

`.github/workflows/ci.yml` 末尾新增 job `publish-image`：

- 條件：`push` 到 `main`；`needs: [api, image, web, e2e]`（測試未全綠不發布）。
- 權限：`contents: read`、`packages: write`，使用內建 `GITHUB_TOKEN`，不需額外 secret。
- 步驟：`docker/setup-buildx-action@v3` → `docker/login-action@v3`（ghcr.io）→ `docker/build-push-action@v6`。
- 標籤：`ghcr.io/erictheng93/wishly-api:${{ github.sha }}` 與 `:latest`；快取 `type=gha`。
- `web-audit` 與 `native` 不在 `needs`，**不擋發布**（`native` 後來因 fmt 修正而通過，見下）。

### 3.2 首次推送遇到的失敗與修正

| Run | 結果 | 原因 | 處理 |
|---|---|---|---|
| `38988109260`（`37f2cb1`） | `api`、`native` 失敗，`publish-image` 被跳過 | 全專案更名後識別字變短，`cargo fmt --check` 排版不符（`apps/api/tests/config_mail.rs`、`notifications.rs`、`apps/web/src-tauri/src/lib.rs`） | 本機 `cargo fmt` 後提交 `40febd1` |
| `39009130541`（`40febd1`） | `api`、`native`、`web`、`e2e`、`image`、`publish-image` 全部成功；**`web-audit` 失敗** | `npm run audit:security` 退出碼 1，**原因尚未查明**（可能是新公告，見 `docs/TODO.md`） | 不擋發布，列入待辦 |

部署使用的映像：`ghcr.io/erictheng93/wishly-api:40febd1f96acf91f63c7383753658f28849b40dd`（run `39009130541` 推送）。

### 3.3 GHCR 維持私有

- 決定：映像保持 Private，Dokploy 使用憑證拉取。
- 憑證：GitHub **Tokens (classic)**，僅勾 `read:packages`。選 classic 的原因是 GHCR 目前不支援 fine-grained token。
- **Dokploy 注意事項（實測）**：Settings → Registry 建立的 Registry **不會**出現在 Application 的 Docker Provider 中；該分頁有自己的 `Registry URL / Username / Password` 欄位，需要再輸入一次憑證。Settings 裡的 Registry 目前沒有被使用。
- PAT 有到期日，到期後 Dokploy 拉映像會 401（影響重新部署，不影響執行中的容器）。到期日需另行記錄（已列入待辦）。

---

## 4. Dokploy `api` 服務

### 4.1 設定

- Provider：**Docker**（由原本的 Git 來源改成；Git 來源會在 VPS 編譯 Rust，已放棄）。
- Image：`ghcr.io/erictheng93/wishly-api:<SHA>`；Registry URL `ghcr.io`；Username `erictheng93`；Password 為 PAT（負責人親自貼入）。
- 回滾方式：把 Image 標籤改回舊的 commit SHA 後重新部署。
- 副本數：維持 **1**（背景工作 `notify::spawn_worker`、`ratelimit::spawn_cleanup` 不是多實例冪等）。
- 不發布任何埠（8080 只在容器網路內）。
- `Autodeploy` 開關目前為開啟狀態；Docker Provider 下它需搭配 webhook 才有意義，**尚未接 CI**（見待辦）。

### 4.2 環境變數鍵名（值不記錄）

非機密（可核對）：

| 鍵 | 值 |
|---|---|
| `APP_ENV` | `production` |
| `BIND` | `0.0.0.0:8080` |
| `RUST_LOG` | `info` |
| `TRUSTED_PROXY` | `cloudflare` |
| `APP_URL` | `https://wishly.tw` |
| `API_BASE_URL` | `https://api.wishly.tw` |
| `MAIL_FROM` | `Wishly <no-reply@wishly.tw>` |
| `S3_BUCKET` / `S3_REGION` | `wishly-media` / `auto` |
| `S3_PUBLIC_BASE` | `https://media.wishly.tw` |
| `S3_ENDPOINT` | `https://<account id>.r2.cloudflarestorage.com` |

機密（僅記錄鍵名，值在密碼管理器與 Dokploy）：`DATABASE_URL`、`OTP_PEPPER`、`OAUTH_SECRET`、`UNSUB_SECRET`、`SHIPPING_ENC_KEY`、`TURNSTILE_SECRET`、`CF_ZONE_ID`（識別碼，非機密但一併放）、`CF_CACHE_PURGE_TOKEN`、`CF_EMAIL_API_TOKEN`、`S3_ACCESS_KEY`、`S3_SECRET_KEY`，另有 `CF_ACCOUNT_ID`。

### 4.3 啟動驗證

- 容器 `wishly-api-qyob9a.1` 狀態 `running`，部署後運行 48 秒以上無重啟。
- production 模式的啟動檢查（缺機密／`TRUSTED_PROXY`／S3 設定即拒絕啟動）通過。
- 日誌：`sqlx` 顯示 `_sqlx_migrations` 已存在（migration 已套用，DB 連線成功）。
- 日誌 WARN：`TRUSTED_PROXY=cloudflare 但 BIND=0.0.0.0:8080 非回送位址：若 API 可被直接存取，CF-Connecting-IP 可被偽造…請只允許經 Cloudflare Tunnel 存取`。這是**預期的提醒**，只要 8080 不對外即成立（驗證見 §5.3）。

### 4.4 一個未解的小疑點

驗證期間我用 Playwright 讀取環境變數編輯器的行數，兩次都得到 18 行，與「已補上 5 個鍵」不符；但容器能在 production 模式啟動（`CF_EMAIL_API_TOKEN` 為必填），且 Purge、R2 上傳均成功，證明實際值存在。**行數量測方式不可靠（編輯器虛擬捲動／遮蔽）**，不應作為驗證依據；以「成功啟動＋功能實測」為準。

---

## 5. Cloudflare Tunnel

### 5.1 設定

- Cloudflare 介面已改版：舊的「Public Hostname」現在位於 Tunnel 的 **Routes** 分頁 → Add route → **Published application**。
- 欄位：Subdomain `api`、Domain `wishly.tw`、Service URL `http://wishly-api-qyob9a:8080`。Cloudflare 自動建立 proxied CNAME。
- **操作陷阱（已避開）**：Domain 下拉預設為帳號中的**另一個專案網域**（`makanmasak.com`）；此帳號還有 `prophetica.net`。切換 Domain 後表單會重新渲染，Service URL 被清空，需重填。送出前以「Full hostname」顯示 `api.wishly.tw` 確認。
- 直連後端容器、不經 Traefik，以降低 SSE 被緩衝的風險。

### 5.2 端點驗證

```
GET https://api.wishly.tw/healthz → 200
GET https://api.wishly.tw/readyz  → 200（含資料庫）
dig api.wishly.tw → Cloudflare IP（104.21.x / 172.67.x，proxied）
```

### 5.3 不可繞過 Tunnel（對應清單 §7.5，部分）

```
curl http://104.105.138.208:8080/healthz → 連線失敗（HTTP 000，逾時）
```

- 已驗證 **8080** 無法從公網直連。
- **尚未驗證 80／443／3000**。先前測試 80/443/3000/8080 都逾時，推測外層防火牆擋住；Docker 其實有發布 `0.0.0.0:3000/80/443`（Dokploy／Traefik 預設），**防火牆規則不可更動**，否則 Dokploy 管理介面會暴露。

### 5.4 觀察到的問題：`/readyz` 對外可達

`/readyz` 會查資料庫，目前任何人都能從公網呼叫（回 200）。清單 §4 要求對外封鎖或限流。**尚未處理**，列入待辦。

---

## 6. 前端（Cloudflare Pages）

### 6.1 專案重建

- 負責人要求「移除重建」Pages 專案。步驟：Dashboard 先移除自訂網域 `wishly.tw` → `wrangler pages project delete wishly-web --yes` → `wrangler pages project create wishly-web --production-branch main --force`（wrangler 預設會把「create」導向 Workers，需 `--force`；之後不需再加）。
- 新 pages.dev 子網域：`wishly-web-5pd`。
- 部署：`NUXT_PUBLIC_TURNSTILE_SITE_KEY=<site key> npm run deploy`（建置時變數；Turnstile **Site key 是公開值**，Secret key 不是）。

### 6.2 自訂網域

- `wishly.tw` 重新綁定由負責人在 Dashboard 完成（wrangler 無此指令）。
- 驗證：`https://wishly.tw/` 回 **200**，`server: cloudflare`、`x-powered-by: Nuxt`；頁面內含 `api.wishly.tw`（建置時 API base 正確）。
- `www.wishly.tw` 無 DNS 記錄（目前不需要；若要支援需另做導向）。

---

## 7. R2（物件儲存）

### 7.1 設定（以 wrangler 完成，帳號 OAuth 已含 R2 權限）

| 項目 | 指令／設定 | 結果 |
|---|---|---|
| bucket | `wrangler r2 bucket create wishly-media --location apac` | 建立成功 |
| CORS | 允許來源 `https://wishly.tw`、方法 `PUT`、標頭 `Content-Type`、`maxAgeSeconds 3600`（`wrangler r2 bucket cors set`） | 成功 |
| Lifecycle | 規則 `expire-incoming`：前綴 `incoming/`、1 天後刪除（`wrangler r2 bucket lifecycle add`） | 成功 |
| 自訂網域 | `wrangler r2 bucket domain add wishly-media --domain media.wishly.tw --zone-id <wishly.tw zone> --min-tls 1.2` | 成功；同時證實使用的 Zone ID 屬於 wishly.tw |
| S3 API Token | 負責人在 Dashboard 建立：**Account API Token**、Object Read & Write、**限定 `wishly-media`** | 值未經過代理程式 |

- 為何選 Account API Token：綁在整個帳號、不因成員離開組織而失效，Cloudflare 頁面標示為 recommended。User API Token 綁個人。
- **不要**重用帳號中既有的「R2 Account Token」（All buckets、Admin Read & Write）：外洩時其他專案的 bucket 都會受影響。

### 7.2 WAF 規則：封鎖 `/incoming/`

- 位置：wishly.tw → Security → Security rules → Custom rules。
- 規則名稱 `Block media incoming prefix`，動作 **Block**，狀態 Active：

```
(http.host eq "media.wishly.tw" and starts_with(http.request.uri.path, "/incoming/"))
```

- 操作陷阱：點 Deploy 的按鈕 ref 在表單重繪後會變，第一次點擊落空（頁面仍停在建立頁、規則未建立）；改點新位置後才成功。以規則清單出現且 Active 為準。
- 驗證：

| 請求 | 結果 |
|---|---|
| `https://media.wishly.tw/incoming/x` | **403**（被擋） |
| `https://media.wishly.tw/other/x` | 404（未被擋，物件不存在） |
| `https://wishly.tw/incoming/x` | 404（其他網域不受影響） |

---

## 8. Email（Cloudflare Email Service）

### 8.1 寄件網域

- 原狀：僅 `makanmasak.com` 已 Onboard；`wishly.tw` 沒有寄信設定（SPF 為 `v=spf1 -all`、無 MX、DKIM 為空）。
- 操作：Email Service → Email Sending → Onboard Domain → 選 `wishly.tw` → Verify DNS records → Activate。
- 新增的 DNS 記錄（全部在 `cf-bounce` 子網域，根網域不動）：

| 類型 | 名稱 | 內容 |
|---|---|---|
| MX | `cf-bounce.wishly.tw` | 優先 57 `route1.mx.cloudflare.net.`、69 `route2…`、91 `route3…` |
| TXT（SPF） | `cf-bounce.wishly.tw` | `v=spf1 include:_spf.mx.cloudflare.net ~all` |
| TXT（DKIM） | `cf-bounce._domainkey.wishly.tw` | `v=DKIM1; h=sha256; k=rsa; p=<公鑰>` |

- 既有記錄**保留**：根網域 SPF `v=spf1 -all`、`_dmarc`（`p=reject; sp=reject; adkim=s; aspf=s`）、萬用字元 `*._domainkey`（空公鑰）。Onboarding 預設也想加一筆 `_dmarc`（`p=reject`），實際查詢結果仍只有原本那一筆，沒有產生重複 DMARC 記錄（重複會使 DMARC 失效）。
- 結果：Email Sending 顯示 **Enabled／Configured**。
- DNS 傳播：權威 DNS（`agustin`/`angela.ns.cloudflare.com`）兩台都已有 DKIM；1.1.1.1 暫時仍回傳萬用字元的空金鑰（快取）。

### 8.2 對齊分析（為何 `p=reject` + 嚴格對齊仍可行）

- SPF 的 Return-Path 是 `cf-bounce.wishly.tw`，與 From 的 `wishly.tw` 在 `aspf=s` 下**不對齊**，SPF 對 DMARC 不通過。
- DKIM 簽章網域 `d=wishly.tw` 與 From 對齊（`adkim=s` 需完全相同），DMARC 靠 **DKIM 通過**。
- 風險：若 Cloudflare 日後改用其他 d= 網域簽章，嚴格對齊會讓信被退。**建議觀察期間先把 DMARC 放寬為 `p=none`**，確認多家信箱（不只 Gmail）正常後再收緊。目前尚未改。

### 8.3 實測

- `POST /api/v1/auth/register` → 202，畫面顯示「驗證碼已寄到 <信箱>」。
- 負責人確認信箱收到 OTP 信（Gmail），並完成註冊（`POST /auth/register/verify` → 200）。
- 這同時證實 `CF_EMAIL_API_TOKEN` 權限足夠、`MAIL_FROM` 網域通過。
- 尚未驗證：Gmail 以外的信箱、是否進垃圾信匣（Gmail 為收件匣與否由負責人判斷，本次未記錄）。

---

## 9. 應用層端對端流程

測試帳號與清單為驗證用資料（見 §12.3）。

### 9.1 登入／註冊

- 登入頁為 Email＋密碼（`/login`），**不是 OTP 登入**；OTP 出現在註冊與忘記密碼流程。
- 註冊頁（`/register`）欄位：顯示名稱、Email、密碼 → 「寄送驗證碼」→ 6 位數驗證碼 → 「完成註冊」。密碼由負責人親自輸入。
- 跨網域驗證：`wishly.tw` → `api.wishly.tw` 的 CORS、cookie、Turnstile 都通過；`GET /me` 未登入時回 401（預期）。

### 9.2 建清單、上傳圖片、發布（`docs/deploy-backend-checklist.md` §7.8）

請求序列（均由瀏覽器實際發出）：

| 步驟 | 請求 | 結果 |
|---|---|---|
| 1 | `POST /api/v1/uploads/presign` | 200 |
| 2 | `PUT https://<account>.r2.cloudflarestorage.com/wishly-media/incoming/<user>/covers/<id>.webp?X-Amz-…`（瀏覽器直傳） | **200**（證實 S3 金鑰、bucket、CORS 正確；CORS 錯誤會在預檢階段失敗） |
| 3 | `POST /api/v1/uploads/confirm` | 200（後端重新編碼、去 EXIF/GPS、寫入公開 key） |
| 4 | `POST /api/v1/wishlists`（公開） | 201 |
| 5 | `POST /api/v1/wishlists/{id}/items` | 201 |
| 6 | `PATCH /api/v1/wishlists/{id}`（發布） | 200 |

驗證結果：

- 公開圖 `https://media.wishly.tw/covers/<id>.webp` → **200**、`image/webp`。
- 同一物件的 `incoming/` 原檔 → **403**（WAF）。
- 清單建立後狀態為 `draft`，公開頁在「至少一個品項＋發布」前回 404（預期）。

---

## 10. CDN 快取與 Purge

### 10.1 發現：公開頁原本沒有被快取

- 後端對 `GET /api/v1/public/wishlists/{slug}` 回 `cache-control: public, s-maxage=10, stale-while-revalidate=60`，但 Cloudflare 回 `cf-cache-status: DYNAMIC`。
- 原因：Cloudflare 預設不快取 JSON，需要 Cache Rule。這條規則在 `docs/deploy-backend-checklist.md` §4 與 README 已列為待辦，但尚未建立。沒有它，每個訪客請求都打到 VPS，Purge 也沒有東西可清。

### 10.2 新增的 Cache Rule

- 位置：wishly.tw → Caching → Cache Rules。
- 名稱 `Cache api public wishlists`；表達式：

```
(http.host eq "api.wishly.tw" and starts_with(http.request.uri.path, "/api/v1/public/"))
```

- 設定：Eligible for cache；Edge TTL = **Use cache-control header if present, bypass cache if not**（**不要**用 Cache Everything + 覆寫 TTL，否則後端的 `s-maxage` 與 `no-store` 語意會被蓋掉）。
- `/api/v1/restricted/*` 不套規則（永遠 `no-store`）。
- 結果：

```
第 1 次  cf-cache-status: MISS
第 2 次  cf-cache-status: HIT  age: 5
第 3 次  cf-cache-status: HIT  age: 7
```

### 10.3 Cache Purge 驗證（`CF_ZONE_ID`、`CF_CACHE_PURGE_TOKEN`；清單 §7.7）

- 第一次嘗試**不算證據**：改設定前快取狀態是 `REVALIDATED`（已超過 10 秒 TTL），變 404 可能只是自然過期。
- 嚴格測試（TTL 內）：清單改為公開 → 等 12 秒 → 請求兩次得 `MISS`、`HIT age:0`、`HIT age:1` → **立刻** `PATCH visibility=private` → 公開頁**立即 404**（`BYPASS`）。
- 結論：快取在 TTL 內被主動清除，Purge 路徑（Zone ID、token 權限與範圍）正確。

---

## 11. SSE（清單 §7.10；Spike S3 的前置）

- 指令：`API_BASE=https://api.wishly.tw SLUG=<slug> ITEM_ID=<uuid> DURATION_MIN=3 TRIGGER_EVERY_S=40 node scripts/sse-probe.mjs`
- 結果（3 分鐘）：

| 指標 | 值 |
|---|---|
| 連線次數／非預期斷線 | 1／**0** |
| `content-type` | `text/event-stream`（未被轉換） |
| TTFB | 1562 ms（單次） |
| 位元組間隔 p50／max／超過 30 秒 | 14982 ms／15477 ms／**0** |
| 觸發事件延遲（認領 POST 完成 → SSE 收到 `item.updated`） | 804 ms（n=1） |

- 心跳約每 15 秒，**沒有被緩衝**；Tunnel → Cloudflare 路徑可傳遞 SSE。
- 「4 次觸發只收到 1 次事件」**不是故障**：第 1 次認領 201 並收到事件；後 3 次回 409（品項數量 1，已被認領）→ 沒有狀態變化所以沒有事件。探針的 `events_missed` 在此測試設計下會誤報。
- **限制**：伺服器每條連線上限 30 分鐘，要驗證越過上限後的重連行為至少需跑 > 30 分鐘；Spike S3 驗收門檻是 120 分鐘。**本次只是短測，不是驗收。**

### 延遲觀察（未定論）

- 量測 `api.wishly.tw/healthz`：POST 耗時 1.9–8.5 秒，TCP connect 最高 5 秒；`cf-ray` 結尾為 **SJC**（美國聖荷西）。
- 對照：`wishly.tw` 連線穩定 0.15 秒；`cloudflare.com` 也出現過一次 2 秒的連線。
- 延遲落在連線階段（發生於請求抵達 Cloudflare 之前），`/healthz` 本身沒有工作量，**判斷為驗證者本機網路的抖動，不是後端或 Tunnel**。但只從單一網路量測，無法下結論；需從其他網路／機器重測，並注意 Cloudflare 把亞洲來源導到 SJC 邊緣是否持續發生。

---

## 12. 發現的問題與風險

### 12.1 機密在驗證過程中外露（需輪替評估）

- 事件：Dokploy `api` 的 Environment 編輯器在某次載入時**未遮蔽**，Playwright 快照把全部 18 行（含值）讀進代理程式的輸出，進入本機對話紀錄與送往模型的上下文。**未寫入 repo，未提交、未推送。**
- 受影響：`DATABASE_URL`（含資料庫密碼）、`OTP_PEPPER`、`OAUTH_SECRET`、`UNSUB_SECRET`、`SHIPPING_ENC_KEY`、`TURNSTILE_SECRET`。此後的檢查改為只讀鍵名／行數。
- 同一份快照也被 Playwright 存成本機檔案（`apps/web/.playwright-cli/page-*.yml`），當時**未被 gitignore 且未追蹤**（從未被提交）。2026-10-17 已刪除該檔，並把 `.playwright-cli/` 加進 `.gitignore`；殘留檢查（以環境變數鍵名與 DB 連線字串樣式全 repo 搜尋）無其他副本。**教訓：對含機密的頁面，不要存快照，只用 `eval` 取鍵名或行數。**
- 對話紀錄（`~/.claude/projects/…/*.jsonl`）仍含這些值，位於使用者本機，無法由代理程式清除；這是建議輪替的原因。
- 為何現在輪替成本最低：資料量極少，且 `SHIPPING_ENC_KEY` 尚無已加密的真實資料（一旦有真實收件資訊就不能再換，沒有輪替機制）。
- 更正先前的說法：**資料庫密碼可以不重建 `db` 服務就輪替**——`POSTGRES_PASSWORD` 只在資料目錄初始化時使用，用 `ALTER ROLE wishly PASSWORD '…'` 並同步更新 Dokploy 的 `DATABASE_URL` 即可（`db` 服務的環境變數同步更新以免日後重建不一致）。
- 各密鑰輪替影響：`OTP_PEPPER`（未使用的 OTP 失效）、`OAUTH_SECRET`（進行中的 OAuth state 失效）、`UNSUB_SECRET`（舊的退訂連結失效）、`TURNSTILE_SECRET`（在 Cloudflare Roll，需同步更新環境變數）。

### 12.2 其他發現

| # | 發現 | 影響 | 處置 |
|---|---|---|---|
| 1 | 公開頁原先沒有 CDN 快取 | 每請求打到 VPS | 已建 Cache Rule（§10） |
| 2 | `/readyz` 對外可達並查 DB | 可被用來放大對 DB 的探測 | 待辦 |
| 3 | `web-audit` CI 失敗，原因未查 | 安全稽核可能有新公告；不擋發布 | 待辦 |
| 4 | `ubuntu-latest` 將在 2026-10-19 遷到 Ubuntu 26（CI 警告） | 可能影響 `api`/`e2e` 的環境 | 待辦（評估是否固定 `ubuntu-24.04`） |
| 5 | Problem `type` URI 不一致：後端為 `https://wishly.tw/problems/…`，`docs/04` 為 `https://api.wishly.tw/problems/…` | 純文件／契約一致性 | 待辦 |
| 6 | DMARC `p=reject` 加嚴格對齊 | 簽章網域變動會導致退信 | 待辦（觀察期先 `p=none`） |
| 7 | Dokploy `Autodeploy` 開啟但無 webhook | 無實際作用，易誤解 | 待辦（接 CI 或關閉） |
| 8 | 帳號與其他專案共用（`makanmasak.com`、`prophetica.net`、多個 R2 bucket、一個 All-buckets Admin R2 token） | 操作時容易選錯網域；外洩影響面大 | 已避開；建議日後權限最小化 |
| 9 | 本機 `apps/api/.env`（gitignored）仍為舊網域；本機開發 DB 需 `docker compose down -v` 才套用新帳號 | 開發環境 | 待辦（未替使用者清資料） |
| 10 | Tauri 識別碼／scheme 已更名但未編譯驗證 | 原生殼 | 待辦（CI `native` job 僅跑 fmt/clippy/lib 測試） |

### 12.3 驗證留下的測試資料（正式環境）

- 一個測試帳號（負責人信箱）。
- 一份清單「部署驗證用清單」（目前可見度「知道連結」）、1 個品項、1 筆認領。
- R2：一張封面圖（公開 key `covers/…webp`）。
- 尚未刪除，由負責人決定。

---

## 13. 對照 `docs/deploy-backend-checklist.md`

| 清單項 | 狀態 |
|---|---|
| §4 `api.<域名>` DNS 為 Tunnel | 已完成 |
| §4 快取規則（public／restricted） | 已完成（SSE `/events` 未額外規則：回應不被快取、未被緩衝，見 §11） |
| §4 WAF／限流；封鎖或限流 `/readyz` | WAF 的 `/incoming/` 已完成；**`/readyz` 未處理** |
| §4 Turnstile widget | 註冊流程已通過 Turnstile；偽造 token 的 403（§7.9）**未測** |
| §4 Email Service 與外部信箱實收 | 已完成（Gmail） |
| §4 前端 Pages 變數與綁定 | 已完成 |
| §7.1 / 7.5（8080） / 7.7 / 7.8 / 7.2 | 已驗證（見上） |
| §7.3 防超賣、§7.4 per-IP 限流、§7.6 CDN 金絲雀、§7.9、§7.11、§7.12、§7.14、§7.15 | **未驗證** |
| §7.10 SSE ≥ 40 分鐘／Spike S3 120 分鐘 | **只做了 3 分鐘短測** |
| §7.13 備份還原演練、§2.1 備份 | **未做（P0）** |
| §6 Dokploy 2FA、SSH 加固、監控告警、`SHIPPING_ENC_KEY` 兩處備份 | **未確認** |
