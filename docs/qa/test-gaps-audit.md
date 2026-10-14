# 測試缺口補強與稽核證據

2026-10-14。以使用者提出的 17 項缺口為範圍；實作與驗證均在本機 PostGIS／Mailpit、Rust stable、Node 22 執行。沒有新增 DB migration。

驗證結果：API 203／203（0 ignored；本次 helper 遷移新增 3 個測試）、前端 97／97、Tauri 2／2、目標瀏覽器旅程 3／3。API 覆蓋率門檻腳本、Rust fmt／clippy 與 Rust audit 通過；npm audit 仍由以下上游 advisory 阻擋。

## 缺口對照

| 項目 | 實作／測試 | 驗證重點 |
| --- | --- | --- |
| 1 同 Idempotency-Key | `apps/api/tests/concurrency.rs` | 第一筆確實阻塞於業務寫入，再送第二筆並確認它等待 idempotency INSERT；兩筆重放同一回應，DB 僅一筆 claim／contribution、一次扣點。 |
| 2 認捐競爭 | 同上 | withdraw 與最後認捐兩種順序；withdraw 與過期／自動退款 tick；轉投與目標最後認捐兩種順序；反向轉投；凍結與認捐。檢查錢包、ledger、contributions、品項快取與採購單。 |
| 3 認領不變式 | `tests/common/mod.rs`、使用完整 migrations 的 DB 測試 | `qty_claimed = SUM(reserved/purchased/delivered claims.qty)`，含 soft-deleted 品項；相關測試結尾對帳。原本只改快取的 fixture 已補實際認領列或改用 API 建立。 |
| 4 多數量競爭 | `tests/concurrency.rs` | needed=5，混合 2／3 件請求與各 2 件；檢查成功量、失敗剩餘量與成功回應快照、最終庫存／claims。允許合法的串行結果，不假設 UPDATE waiter 的 FIFO。 |
| 5 刪除／封存競爭 | 同上 | quantity／crowdfund × force delete／archive × 兩種先後；另外驗證刪品項與改認領的 item→claim 鎖序、保留 delivered 數量。 |
| 6 CDN purge | `tests/cdn_invalidation.rs` | 接收實際 Cloudflare mock 請求：private、DELETE／PATCH archive、moderation hidden；檢查 URL、DB 變更與公開端點不再可讀。 |
| 7 DB 約束 | `tests/migration_constraints.rs` | 直接非法 INSERT／UPDATE／DELETE，檢查 SQLSTATE／constraint 與最終資料；數量範圍、wallet／ledger 非負值、帳本只增不改、眾籌金額與狀態形狀。不是所有 CHECK 的完整枚舉。 |
| 8 Profiles | `tests/profiles.rs`、既有 `social.rs` | handle 大小寫；專屬測試涵蓋匿名／本人，既有社群測試涵蓋好友的欄位與清單可見性；未公開／封存／下架／已刪清單，以及已刪帳號。 |
| 9 Presign | `tests/uploads.rs` | 真實端點的 MIME／用途／大小邊界、簽名 headers、incoming object_key 與 user 綁定、跨使用者 confirm 被拒。 |
| 10 SMTP worker | `tests/smtp_worker.rs`、CI API Mailpit service | 一般測試即執行，不再 ignore；直接 tick，檢查 sent／attempts／sent_at 與 Mailpit 的實際收件、隱私內容、不重寄。 |
| 11 品質閘門 | `.github/workflows/ci.yml`、`scripts/{audit-api,coverage-api}.sh`、兩份 coverage baseline | Rust fmt／clippy `-D warnings`、Rust／npm audit、API／Vitest 覆蓋率門檻與 artifact；原生測試新增 macOS CI job。npm audit 外部阻擋見下。 |
| 12 共用 helper | `tests/common/mod.rs` | 依共用骨架統一 `call`／`call_app`、`TestUser`／`ListOpts`、headers、明確數量的 fixtures、ledger 發點與三種對帳。只有原始 IP／Bearer／API 建立 fixture 保留必要特化；逐檔測試後 commit，無 legacy alias。 |
| 13 功能命名 | `tests/otp.rs`、`line_oauth.rs`、`wishlist_items.rs`、`guest_lifecycle.rs`、`notifications.rs` 等 | slice3／slice45／gaps 拆成對應功能；regress 改為 auth_regressions；原測試保留，忽略 SMTP 由更強的正常測試接替。 |
| 14 時間 flaky | `src/dashboard.rs`、SSE／identity／admin／notification tests | 回應 SSE 前完成 LISTEN；移除 800ms sleep 與 10ms 輪詢 sleep。用實際 DB blocking predicate＋失敗上限；通知在同交易固定 now() 並檢查精確台北 09:00 排程。 |
| 15 前端單元 | `tests/use{Auth,Guest,WishlistEvents}.test.ts`、`contributionSheets.test.ts` | 身份讀取／錯誤與登出、token／Cookie 優先序、連續兩次 SSE error 改輪詢及復原／cleanup、ClaimSheet／FundSheet 驗證與 pending／成功／錯誤狀態。 |
| 16 qa 定位 | README「測試」 | 明確標註 `qa/*.mjs` 為手動探索腳本，不屬 CI 自動測試。 |
| 17 Tauri | `src-tauri/src/lib.rs`、native CI job | keyring 自帶 mock 的共享記憶體 builder；實際 get／set／replace／clear、非法 token 保留原值、錯誤傳遞，沒有操作使用者 OS keychain。 |

## 補測發現的實際問題

- 認領缺 wishlist 共享鎖，封存先提交後仍可能寫入：補清單→品項鎖序。
- 強制刪品項先改 claims、再鎖 item，與認領修改反向：改為 wishlist→item→claim；刪除保留 delivered 的 qty 帳。
- 刪帳號的 users 鎖與 claim／pledge／通知 FK 形成死鎖：creator 先鎖有效 actor 與 owner，再鎖清單，等待時已刪 actor 不能再寫入。新增 own claim、guest claim、own pledge 與另一清單的 actor 刪除競爭測試。
- SSE 首個通知可能在 LISTEN 啟動前遺失：先完成 LISTEN。Pool 關閉時 PgListener 原生回傳關閉錯誤，重連流程須終止；收尾有上限與連線回收檢查。
- ClaimSheet 編輯既有認領時暱稱欄位隱藏，驗證卻仍要求暱稱：編輯模式略過新訪客暱稱要求。
- 登入者快速按贊助時 `/me` 未完成即被導去登入：`onFund` 等待既有 `meReady`。E2E 用真實 `/me` 請求 Promise 屏障穩定重現，修正後完整旅程通過。

## Mutation 與最終 DB 證據

併發、前端、原生與 SSE 的結果／失敗輸出在 [test-gap-evidence](test-gap-evidence/)；併發結果為 [concurrency-mutations.json](test-gap-evidence/concurrency-mutations.json)。所有暫時 mutations 已還原。

| Mutation | 捕捉到的失敗 |
| --- | --- |
| 移除 claim wishlist FOR SHARE | archive 已先提交，最終 DB qty_claimed 仍由 0 變 2。 |
| 移除 pledge wallet FOR UPDATE | frozen wallet 最終 balance 由 1000 變 800。 |
| reallocate 改來源品項優先 | high→low 先鎖 high，違反兩個方向都先鎖較小 id 的實際 blocking predicate。 |
| 移除 delete 的 item FOR UPDATE | 真實 PostgreSQL `deadlock detected`，更新認領回 500。 |
| 移除 idempotency ON CONFLICT | claim、pledge 第二筆均回 500，未能重放／409。 |
| 停用 CDN purge | 第一個 private 案例的 mock 沒收到 purge；archive／hidden 有正常測試，未各自執行 mutation。 |
| 移除 qty／wallet CHECK | 非法資料寫入成功，預期 DB error 的斷言失敗。 |
| 移除 ledger immutable trigger | ledger UPDATE 成功，預期拒絕的斷言失敗。 |
| SSE fallback 改成三次 error | 第二次 error 後仍 idle，輪詢測試失敗。 |
| keychain get 吞錯誤 | 回 Ok(None)，錯誤傳遞測試失敗。 |
| 移除贊助的 meReady 等待 | 真實身份查詢仍在屏障中，登入使用者被導去 login，對話框／URL 斷言失敗。 |

CDN、CHECK 與 ledger trigger 的 mutation 失敗輸出留在本次工具對話紀錄，沒有另存原始 stdout 檔案。

反向轉投兩筆**不能同時合法**：來源必須 expired、目標必須 open。此案例驗證鎖序、兩筆 409 與 DB 不漂移；不能把該 mutation 稱為合法交換時的資金損壞證據。

併發檔在預設測試執行緒下 17／17 通過，再連跑五次共 85／85 通過。SQLx 0.8 的測試 pool 共用 20 個連線；test-only Semaphore(2) 限制同時跑的案例數，**每個案例內的請求仍真正併發**，並用 `pg_stat_activity`／`pg_blocking_pids` 確認抵達衝突點。不要對同一 DB 同時跑兩份相同 SQLx test path；測試資料庫名稱由 test path 決定。

新增／調整的 API 與前端單元測試沒有固定 sleep。10／15 秒的 timeout 是缺少事件／死鎖的失敗上限；前端輪詢採 fake timers，E2E 身份載入採 Promise 屏障。

## 覆蓋率與閘門

API 基準為 [coverage-baseline.json](../../apps/api/tests/coverage-baseline.json)，涵蓋 API 自有 Rust 程式碼，排除 tests／examples 與外部 dependencies；保護 lines 91.28%、functions 88.25%、regions 87.15%。原生殼目前是 command unit tests，沒有宣稱原生介面／OS keychain 的整體覆蓋率。

前端基準為 [coverage-baseline.json](../../apps/web/tests/coverage-baseline.json)：lines 52.67%、statements 49.85%、functions 49.15%、branches 49.62%。範圍為 composables、utils、components，包含未測元件的零覆蓋檔；不含 route pages。Node 22／24 的結果相同。門檻固定，不會在 CI 自動調低。

一次性的 Rust 全專案格式化與既有 clippy 清理，是啟用 fmt／`-D warnings` 閘門所需；除上述修正外，其餘 production 差異主要是格式、SQL row tuple alias、冗餘 cast／Vec 清理。

## 外部阻擋

`npm audit --audit-level=high` 仍失敗，沒有設定 continue-on-error 或忽略 advisory：

- `braces 3.0.3`：[GHSA-vfj7-8cjw-p6xm](https://github.com/advisories/GHSA-vfj7-8cjw-p6xm)。
- `node-forge 1.4.0`：[GHSA-86w9-cpqp-85rv](https://github.com/advisories/GHSA-86w9-cpqp-85rv)。

兩者由 Nuxt→Nitro 間接引入，查核當時 npm registry 最新版本仍受影響。已更新 Nuxt 4.6.1、Devtools 3.4.3、Wrangler 4.151.0，消除 critical；audit 的 aggregate 仍有 11 high。精確 resolver 資訊見 [npm-advisories.json](test-gap-evidence/npm-advisories.json)。

API 鎖檔的 RSA `RUSTSEC-2023-0071` 屬未啟用的 SQLx MySQL optional dependency。`audit-api.sh` 先檢查所有平台的實際依賴樹，只有 RSA 不在樹中才略過該 advisory；一旦啟用 RSA 會恢復完整 audit 並失敗。原生 audit 無 vulnerability，仍有上游 informational warnings。

## 重跑

```sh
docker compose up -d db mailpit
export DATABASE_URL=postgres://wishsync:wishsync@localhost:5432/wishsync
export APP_ENV=dev
cargo fmt --manifest-path apps/api/Cargo.toml --check
cargo clippy --manifest-path apps/api/Cargo.toml --all-targets -- -D warnings
sh scripts/audit-api.sh
sh scripts/coverage-api.sh
cargo test --manifest-path apps/api/Cargo.toml --test concurrency
cargo test --manifest-path apps/web/src-tauri/Cargo.toml --lib
cd apps/web
npm run typecheck
npm run test:coverage
npm run build
npm audit --audit-level=high
npx playwright test guest-journey.spec.ts crowdfund.spec.ts --project=desktop-chromium
```

## 共用 helper 遷移追加驗證

遷移的起點為 `b804d30`，先建立骨架並搬 hardening，再搬原 gaps 的功能檔、visibility、原 slice3／slice45 的功能檔，其餘由兩個 worker 分檔執行。每檔測試通過後各自 commit；未改產品 API 行為。`claims.rs` 與 `crowdfund_pledge.rs` 保留隔離 Router，併發測試 clone 同一個 Router，再呼叫 `call_app`。原有 qty、NULL email、title、身份 fixture 與原始 Cookie 格式均保留；需要固定 slug 的案例仍指定原 slug，CDN／profile 案例改用實際 TestList.slug 做斷言。

`test_helpers.rs` 的 3 個測試涵蓋 fixture 選項、password 真實 unlock、三種有效認領狀態與 cancelled／expired 排除、三種對帳漂移的拒絕、JSON／空／純文字回應和 malformed JSON 拒絕。`wait_until` 以條件與逾時上限運作，沒有固定 sleep。秘密客 privacy fixture 與 quantity fixture 也經單一 wishlist／item 實作，needed／qty 明確指定。

唯一完整財務對帳的例外是 `contribution_visibility_migration_preserves_existing_scope`：它只套用 0001–0005 後驗證 0006 backfill，刻意未扣錢包／累加品項；保留原本 before／after scope assertions，並在測試內註明。舊 schema 尚無 closes_at，因此只在此案例用最小 inline SQL 建清單。

追加的 [mutation 記錄](test-gap-evidence/helper-mutations.json) 與 5 份 raw logs 均為實際跑測試所得，mutant 執行後以 SHA-256 確認原始碼還原：

- 停用認領、錢包、認捐品項任一對帳斷言，負 fixture 測試都會失敗，避免 helper 變成 no-op 還全綠。
- 移除 pledge 的錢包 FOR UPDATE：凍結錢包最終 balance 變成 800，而預期為 1000；由最終 DB 狀態殺死 mutant。
- 改成來源品項優先鎖：預期的 DB blocked statement 不再成立，測試在診斷上限失敗。此追加 log 驗證鎖順序；真實死鎖的資料庫證據仍見上一輪 `wishlist-mutation-delete-deadlock-evidence.txt`，不把本次的等待失敗宣稱為新死鎖證據。

還原後完整 API coverage gate：203 passed、0 failed、0 ignored；lines 91.289%、functions 88.255%、regions 87.157%，覆蓋的行／函式／region 數與原基準完全相同，沒有調低門檻。原始結果為 [完整測試與 coverage log](test-gap-evidence/helper-migration-final.txt)；Rust fmt、clippy `-D warnings` 與 diff whitespace 檢查也通過。上一輪的 200-test artifacts 保持原樣，新增證據區分本次 203-test 結果。
