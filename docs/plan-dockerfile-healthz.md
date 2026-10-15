# 實作計畫：後端 Dockerfile 與 `/healthz`、`/readyz`

> **部署平台更新（2026-10-15，PRD D49）：後端改部署在 Linode VPS + Dokploy + Cloudflare Tunnel，不再使用 Cloud Run。本計畫中「Cloud Run」「Cloud Scheduler」「Cloud Run 探針」的字樣為歷史脈絡；Dockerfile、探針與優雅關閉的設計不變，部署設定改見 [deploy-backend-checklist](deploy-backend-checklist.md) §2.4–§2.5。**
>
> **狀態（2026-10-15）：§2、§3、§4 已實作並驗收；實作與驗證證據見 commit `e2593f5`、`bdc3daf`、`99513dc`。**

對應 [deploy-backend-checklist §0.1、§0.2](deploy-backend-checklist.md)。實作由負責人進行，本文件同時是稽核的驗收標準。
範圍刻意只含這兩項；背景工作排程（§0.3）、多實例冪等（§0.4）另案，但本計畫的 Dockerfile 與探針設定不得與其衝突。

## 1. 事實基礎（查核自程式碼）

| 事實 | 來源 | 對計畫的影響 |
|---|---|---|
| 工具鏈固定 Rust `1.99.0` | `rust-toolchain.toml`、CI `dtolnay/rust-toolchain@1.99.0` | builder image 版本必須一致 |
| crate 在 `apps/api`，**沒有 workspace Cargo.toml**；`Cargo.lock` 在 `apps/api/` | `ls` | 建置用 `--manifest-path apps/api/Cargo.toml`，lock 隨之 `--locked` |
| migrations 以 `sqlx::migrate!("../../db/migrations")` **編譯期內嵌** | `main.rs:22` | **build context 必須是 repo 根目錄**且含 `db/migrations`；Dockerfile 放 `apps/api/Dockerfile` 但以根為 context |
| 無 `query!` 系列巨集（無 `.sqlx/` 離線快取） | grep | 建置不需要連 DB |
| TLS 全為 rustls（sqlx `tls-rustls`、reqwest `rustls-tls`）、`image` 無原生相依 | `Cargo.toml` | 不需 openssl／libpq；runtime 只需 glibc |
| 單一 binary `wishsync-api`（`src/main.rs`）；`examples/seed_demo.rs` 不該進 image | `Cargo.toml`、`ls` | `cargo build --release --bin wishsync-api` |
| 預設 `BIND=127.0.0.1:8080`，production 容器需明確設 `0.0.0.0:8080` | `config.rs`、README | 不在 image 內寫死，由部署平台（Dokploy）環境變數設定；但 Dockerfile 要 `EXPOSE 8080` 並在文件註明 |
| `tracing_subscriber::EnvFilter::from_default_env()`，**未設 `RUST_LOG` 時預設只輸出 ERROR** | `main.rs:6-8` | 部署必須設 `RUST_LOG=info`，否則 Cloud Logging 幾乎沒有內容。可在 Dockerfile `ENV RUST_LOG=info` 當預設 |
| middleware 順序：`read_only`（GET/HEAD/OPTIONS 視為安全）→ CORS → problem_json → security_headers | `lib.rs:app()` | `/healthz` 為 GET，不會被唯讀擋；會被加上 `Cache-Control: no-store`（正好） |
| 路由全部 `nest("/api/v1", …)` | `lib.rs` | 探針掛在**根路徑**（不在 `/api/v1` 下），避免與公開 API 契約混在一起 |
| `axum::serve` 無 graceful shutdown；tokio 已啟用 `signal` feature | `main.rs`、`Cargo.toml` | 見 §4 選配 |

## 2. Dockerfile

位置：`apps/api/Dockerfile`；建置指令（repo 根）：`docker build -f apps/api/Dockerfile -t wishsync-api .`

### 設計
1. **builder**：`rust:1.99.0-bookworm`（與 runtime 同為 Debian 12 glibc，避免 glibc 版本不符）。
2. **依賴快取層**：先 COPY `apps/api/Cargo.toml`、`Cargo.lock`，建立空的 `src/main.rs`／`src/lib.rs` 預先 `cargo build --release` 一次，再 COPY 真實原始碼。理由：依賴量大（sqlx、image、reqwest），不分層每次改一行都全編譯。**不引入 cargo-chef**（多一個工具，收益相同）。
3. COPY 範圍：`apps/api/`（`src`、`Cargo.*`）與 `db/migrations/`，維持相對路徑 `../../db/migrations` 成立（builder 內路徑要保持 `/app/apps/api` 與 `/app/db/migrations`）。
4. `cargo build --release --locked --bin wishsync-api`；`--locked` 確保不偷升級依賴。
5. **runtime**：`gcr.io/distroless/cc-debian12:nonroot`（含 glibc、CA 憑證，無 shell，非 root）。只 COPY 一個 binary。
6. `ENV RUST_LOG=info`、`EXPOSE 8080`、`USER nonroot`、`ENTRYPOINT ["/wishsync-api"]`。
7. **不寫 `HEALTHCHECK`**：distroless 無 curl／shell；探針由部署平台與外部監控設定（見 §3.4），不用 image 內建。
8. 不放任何機密；`APP_ENV` 不在 image 內設定（未設即 production，fail closed）。

### 同時新增 `.dockerignore`（repo 根）
排除：`**/target`、`**/node_modules`、`apps/web`、`docs`、`.git`、`.github`、`design`、`*.md`（`db/migrations` 與 `apps/api` 必須保留）。理由：context 在根，不排除會把前端與 target 數 GB 送進 daemon。

### 驗收
- [ ] `docker build` 在乾淨環境成功；第二次只改 `src/*.rs` 時依賴層命中快取（輸出含 `CACHED`）。
- [ ] `docker image inspect` 大小合理（預期 < 120 MB），且 `docker run --rm --entrypoint sh` 失敗（確認無 shell）。
- [ ] `docker run` 不帶機密 → 啟動即退出並印出「production 需設定 …」（fail-closed 在 image 內成立）。
- [ ] 帶 `APP_ENV=dev`、`BIND=0.0.0.0:8080`、`DATABASE_URL` 指向 compose 的 db → 能啟動、套用 migrations、`curl :8080/healthz` 200。
- [ ] 以 `id` 或 `docker inspect` 確認執行身分非 root。
- [ ] image 內不含 `target/`、`.env`、`docs/`（`docker run` 無 shell，改用 `docker create` + `docker export | tar -t` 檢查）。
- [ ] CI 新增一個 job 僅執行 `docker build`（不 push），避免 Dockerfile 悄悄壞掉；推送 Artifact Registry 屬 CD，另案。

## 3. `/healthz` 與 `/readyz`

### 3.1 語意（兩個端點，不合併）
| 端點 | 用途 | 檢查 | 失敗代表 |
|---|---|---|---|
| `GET /healthz` | **liveness**／startup | 不碰 DB，行程能回應即 200 | 行程卡死 → 重啟 |
| `GET /readyz` | readiness／監控 | `SELECT 1`（pool 取連線＋查詢，**2 秒逾時**） | DB 不可達 → 503，不重啟 |

不合併的原因：若 liveness 依賴 DB，DB 短暫故障會讓編排層把服務不斷重啟，把局部故障放大成全面停擺。

### 3.2 實作位置與行為
- 檔案：新增 `apps/api/src/health.rs`，提供 `routes()`；在 `lib.rs::app()` 以 **`.merge(health::routes())` 掛在 `/api/v1` nest 之外**。（CLAUDE.md：新增模組只改 `lib.rs`。）
- `/healthz`：回 `200`，body `{"status":"ok"}`（`application/json`）。
- `/readyz`：
  - 成功 `200 {"status":"ok"}`。
  - 失敗 `503 {"status":"unavailable"}`。**不要回傳錯誤訊息、連線字串或 sqlx 錯誤內容**（探針端點可能被外部看到）；錯誤只寫 `tracing::warn!`。
  - 逾時以 `tokio::time::timeout(Duration::from_secs(2), …)` 包住，避免探針自己卡住。
  - 唯讀模式（`system_flags.read_only`）**不影響** readyz：唯讀仍是可服務狀態。
- 不需要登入、不讀 cookie、不寫 DB、不產生審計紀錄。
- 回應會自動帶 `Cache-Control: no-store` 與 CDN no-store 標頭（`security_headers`），不需額外處理。

### 3.3 不要做的事
- 不回報版本號、commit、依賴、DB 狀態細節（資訊洩漏；需要時另開營運用端點並要求登入）。
- 不在 `/healthz` 做任何 IO。
- 不把探針放進 OpenAPI／`docs/04` 公開契約；於 `docs/04` 的「內部端點」旁以一行註記即可。

### 3.4 部署設定（寫入 checklist §2.3，不是程式碼）
- 啟動探針（若平台支援）：`GET /healthz`，`failureThreshold` 配合 migration 時間（啟動時會先跑 `sqlx::migrate!`，大 migration 會延後 listen；`bind` 在 migrate 之後，所以探針在 migrate 完成前連線被拒屬正常）。
- **liveness probe**：`GET /healthz`。
- 外部監控（Cloudflare Health Check／Uptime）打 `/readyz`。
- **Cloudflare WAF 規則：對外封鎖或限流 `/readyz`**（它會查 DB，公開暴露可被當作放大器）。平台內部探針直連容器，不經 Cloudflare，不受此規則影響。`/healthz` 無 IO，可開放。

### 3.5 測試（`apps/api/tests/health.rs`，沿用 `#[sqlx::test(migrations = "../../db/migrations")]` 與既有 oneshot 寫法）
- [ ] `GET /healthz` → 200，body 為 `{"status":"ok"}`，帶 `cache-control: no-store`。
- [ ] `GET /readyz`（DB 正常）→ 200。
- [ ] 關閉 pool（`pool.close()`）後 `GET /readyz` → 503，body **不含**任何 DB 錯誤字串；同時 `GET /healthz` 仍 200（證明兩者獨立）。
- [ ] 唯讀旗標為 true 時兩者仍 200。
- [ ] `/api/v1/healthz` → 404（確認沒有掛錯位置）。
- 覆蓋率：專案有 `scripts/coverage-api.sh` 與 `scripts/test_coverage.py` 閘門，新增檔案的行需被測試覆蓋；實作後跑 `sh scripts/coverage-api.sh` 確認不降低基線。
- `scripts/mutate.sh`：此處不涉及鎖序／併發 SQL，**不需新增 mutant**（依 CLAUDE.md 規範）。

## 4. 選配但建議一併處理：SIGTERM 優雅關閉

換版／停止容器時（Dokploy／`docker stop`）送 SIGTERM，預設 10 秒後強殺。現況 `axum::serve` 無 graceful shutdown，進行中的認領 POST 會被切斷（有 Idempotency-Key，客戶端重試可恢復，但體驗差）。
- 做法：`axum::serve(...).with_graceful_shutdown(shutdown_signal())`，`shutdown_signal` 等待 `tokio::signal::unix::signal(SignalKind::terminate())` 與 ctrl-c（tokio 已啟用 `signal`）。
- 注意 SSE 單連線最長 30 分鐘，graceful shutdown 會等連線結束 → 必須搭配 `with_graceful_shutdown` 之外的強制上限（容器的停止寬限期會兜底），不可無限等待。
- 這項會觸及 `main.rs`，與 §0.3 的排程決策（背景工作是否要隨 SIGTERM 停止）相關，**建議與 §0.3 一起做**。若只想先交付探針與 image，可延後，不影響本計畫驗收。

## 5. 建議提交切分（atomic）
1. `feat(api): 新增 /healthz 與 /readyz` — `health.rs`、`lib.rs` 一行 merge、`tests/health.rs`。
2. `build(api): 新增 Dockerfile 與 .dockerignore`。
3. `ci: 驗證 Docker 映像可建置`。
4. `docs: 探針與映像部署設定寫入 checklist §2.3、TODO 勾選`。

順序理由：先有端點，Dockerfile 驗收時才能用 `curl` 驗證。

## 6. 風險與未知
- `rust:1.99.0` 官方映像的標籤是否存在需在實作時確認（以 `docker pull` 為準）；不存在時改用 `rustup` 在 builder 內安裝指定版本。
- distroless/cc 的 glibc 與 builder 的 glibc 需同為 Debian 12；若日後 base 升級，兩邊要同步。
- 尚未量測冷啟動時間（含 migration），啟動探針門檻需待首次在 VPS 實測後調整。
- 本計畫不含映像漏洞掃描與簽章（供應鏈），上線前另列。
