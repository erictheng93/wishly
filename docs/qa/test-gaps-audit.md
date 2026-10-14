# 測試缺口補強與稽核證據

2026-10-14。以使用者提出的 17 項缺口為範圍；實作與驗證均在本機 PostGIS／Mailpit、Rust stable、Node 22 執行。沒有新增 DB migration。

驗證結果：API 205／205（0 ignored；helper 遷移新增 3 個測試，本輪新增 2 個 CDN trigger 測試）、前端 97／97、Tauri 2／2、目標瀏覽器旅程 3／3。API 覆蓋率門檻腳本、Rust fmt／clippy 與 Rust audit 通過；npm audit 仍由以下上游 advisory 阻擋。

## 缺口對照

| 項目 | 實作／測試 | 驗證重點 |
| --- | --- | --- |
| 1 同 Idempotency-Key | `apps/api/tests/concurrency.rs` | 第一筆確實阻塞於業務寫入，再送第二筆並確認它等待 idempotency INSERT；兩筆重放同一回應，DB 僅一筆 claim／contribution、一次扣點。 |
| 2 認捐競爭 | 同上 | withdraw 與最後認捐兩種順序；withdraw 與過期／自動退款 tick；轉投與目標最後認捐兩種順序；反向轉投；凍結與認捐。檢查錢包、ledger、contributions、品項快取與採購單。 |
| 3 認領不變式 | `tests/common/mod.rs`、使用完整 migrations 的 DB 測試 | `qty_claimed = SUM(reserved/purchased/delivered claims.qty)`，含 soft-deleted 品項；相關測試結尾對帳。原本只改快取的 fixture 已補實際認領列或改用 API 建立。 |
| 4 多數量競爭 | `tests/concurrency.rs` | needed=5，混合 2／3 件請求與各 2 件；檢查成功量、失敗剩餘量與成功回應快照、最終庫存／claims。允許合法的串行結果，不假設 UPDATE waiter 的 FIFO。 |
| 5 刪除／封存競爭 | 同上 | quantity／crowdfund × force delete／archive × 兩種先後；另外驗證刪品項與改認領的 item→claim 鎖序、保留 delivered 數量。 |
| 6 CDN purge | `tests/{cdn_invalidation,account_deletion,closes_at}.rs` | 五個呼叫點都有實際 Cloudflare mock 請求與逐點 mutation。刪帳號檢查 slug、DB private／archived、origin 404；close_due 檢查只含到期 slug、非到期／draft 排除、DB closed 與 origin 狀態。 |
| 7 DB 約束 | `tests/migration_constraints.rs` | 直接非法 INSERT／UPDATE／DELETE，檢查 SQLSTATE／constraint 與最終資料；數量範圍、wallet／ledger 非負值、帳本只增不改、眾籌金額與狀態形狀。不是所有 CHECK 的完整枚舉。 |
| 8 Profiles | `tests/profiles.rs`、既有 `social.rs` | handle 大小寫；專屬測試涵蓋匿名／本人，既有社群測試涵蓋好友的欄位與清單可見性；未公開／封存／下架／已刪清單，以及已刪帳號。 |
| 9 Presign | `tests/uploads.rs` | 真實端點的 MIME／用途／大小邊界、簽名 headers、incoming object_key 與 user 綁定、跨使用者 confirm 被拒。 |
| 10 SMTP worker | `tests/smtp_worker.rs`、CI API Mailpit service | 一般測試即執行，不再 ignore；直接 tick，檢查 sent／attempts／sent_at 與 Mailpit 的實際收件、隱私內容、不重寄。 |
| 11 品質閘門 | `.github/workflows/ci.yml`、`scripts/{audit-api,coverage-api}.sh`、兩份 coverage baseline | Rust fmt／clippy `-D warnings`、Rust audit、獨立 web-audit job、API／Vitest 覆蓋率門檻與 artifact；原生測試新增 macOS CI job。npm audit 外部阻擋見下。 |
| 12 共用 helper | `tests/common/mod.rs` | 依共用骨架統一 `call`／`call_app`、`TestUser`／`ListOpts`、headers、明確數量的 fixtures、ledger 發點與三種對帳。只有原始 IP／Bearer／API 建立 fixture 保留必要特化；逐檔測試後 commit，無 legacy alias。 |
| 13 功能命名 | `tests/otp.rs`、`line_oauth.rs`、`wishlist_items.rs`、`guest_lifecycle.rs`、`notifications.rs` 等 | slice3／slice45／gaps 拆成對應功能；regress 改為 auth_regressions；原測試保留，忽略 SMTP 由更強的正常測試接替。 |
| 14 時間 flaky | `src/dashboard.rs`、SSE／identity／admin／notification tests | 回應 SSE 前完成 LISTEN；移除固定 800ms 等待。共用 DB 輪詢採 predicate＋5ms 降噪間隔＋失敗上限；通知在同交易固定 now() 並檢查精確台北 09:00 排程。 |
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
| reallocate 改來源品項優先 | 原先捕捉鎖序 predicate；2026-10-15 runner 進一步形成真實死鎖，觀察 isolated DB deadlocks 0→1 並驗 final DB 完整回滾。 |
| 移除 delete 的 item FOR UPDATE | 真實 PostgreSQL `deadlock detected`，更新認領回 500。 |
| 移除 idempotency ON CONFLICT | claim、pledge 第二筆均回 500，未能重放／409。 |
| 停用 CDN purge | 五個呼叫點（private、archive、hidden、account delete、close_due）逐一停用後，對應 mock 接收斷言皆失敗；全部可由 scripts/mutate.sh 重跑。 |
| 移除 qty／wallet CHECK | 非法資料寫入成功，預期 DB error 的斷言失敗。 |
| 移除 ledger immutable trigger | ledger UPDATE 成功，預期拒絕的斷言失敗。 |
| SSE fallback 改成三次 error | 第二次 error 後仍 idle，輪詢測試失敗。 |
| keychain get 吞錯誤 | 回 Ok(None)，錯誤傳遞測試失敗。 |
| 移除贊助的 meReady 等待 | 真實身份查詢仍在屏障中，登入使用者被導去 login，對話框／URL 斷言失敗。 |

上一輪帳號刪除／close_due 沒有 CDN trigger 測試，不只是少了 mutation 記錄；先前表述不完整。本輪已補齊。CDN、CHECK、ledger 的可重跑 runner、精確失敗訊息、patch、SHA 與 raw stdout 見 [稽核修正證據](test-gap-evidence/audit-followup/mutations.json)。

反向轉投兩筆**不能同時合法**：來源必須 expired、目標必須 open。此案例驗證鎖序、兩筆 409 與 DB 不漂移；不能把該 mutation 稱為合法交換時的資金損壞證據。

併發檔在預設測試執行緒下 17／17 通過，再連跑五次共 85／85 通過。SQLx 0.8 的測試 pool 共用 20 個連線；test-only Semaphore(2) 限制同時跑的案例數，**每個案例內的請求仍真正併發**，並用 `pg_stat_activity`／`pg_blocking_pids` 確認抵達衝突點。不要對同一 DB 同時跑兩份相同 SQLx test path；測試資料庫名稱由 test path 決定。

測試不以固定等待時間判定成功；wait_until 的 5ms sleep 只降低輪詢頻率。5／10／15 秒的 timeout 是缺少事件／死鎖的失敗上限；前端輪詢採 fake timers，E2E 身份載入採 Promise 屏障。

## 覆蓋率與閘門

API 基準為 [coverage-baseline.json](../../apps/api/tests/coverage-baseline.json)，涵蓋 API 自有 Rust 程式碼，排除 tests／examples 與外部 dependencies。2026-10-15 保留實測 counts，加入每項 0.5 個百分點的明確餘裕；門檻直接由 covered／total 計算，為 lines 90.789157%、functions 87.755034%、regions 86.656589%。允許下降最多 0.5 個百分點，並非每 PR 零下降保證；CI 不自動改基準或餘裕。Rust 與 Docker／CI 固定 1.99.0，cargo-llvm-cov 固定 0.9.1；coverage script 若版本不同會先拒絕。原生殼目前是 command unit tests，沒有宣稱原生介面／OS keychain 的整體覆蓋率。

更新基準時，以固定工具版本重新執行 `sh scripts/coverage-api.sh`，取 `apps/api/target/coverage.json` 的 `data[0].totals`，把三項 `covered`／`count` 記入 baseline 的 `covered`／`total`。若工具改版，同步更新 toolchain 設定、CI 安裝版本與 baseline 的 toolchain，再保存重測證據。PR 必須說明範圍、刪除已覆蓋程式碼等比例變動原因；不能只因 gate 紅燈就降低 counts。餘裕是另行審核的政策值，不在重新量測時自動擴大。`python3 scripts/test_coverage.py` 驗證門檻傳遞與版本不符拒絕。

前端基準為 [coverage-baseline.json](../../apps/web/tests/coverage-baseline.json)：lines 52.67%、statements 49.85%、functions 49.15%、branches 49.62%。範圍為 composables、utils、components，包含未測元件的零覆蓋檔；不含 route pages。Node 22／24 的結果相同。門檻固定，不會在 CI 自動調低。

一次性的 Rust 全專案格式化與既有 clippy 清理，是啟用 fmt／`-D warnings` 閘門所需；除上述修正外，其餘 production 差異主要是格式、SQL row tuple alias、冗餘 cast／Vec 清理。

## 外部阻擋

`npm audit --audit-level=high` 仍失敗；本輪已移到獨立 `web-audit` job，保持失敗與 JSON artifact，不再遮蔽 `web` 的 typecheck／test／build。沒有 continue-on-error 或忽略 advisory：

- `braces 3.0.3`：[GHSA-vfj7-8cjw-p6xm](https://github.com/advisories/GHSA-vfj7-8cjw-p6xm)。
- `node-forge 1.4.0`：[GHSA-86w9-cpqp-85rv](https://github.com/advisories/GHSA-86w9-cpqp-85rv)。

兩者由 Nuxt→Nitro 間接引入，上一輪查核時 npm registry 最新版本仍受影響。已更新 Nuxt 4.6.1、Devtools 3.4.3、Wrangler 4.151.0，消除 critical；audit 的 aggregate 仍有 11 high。精確 resolver 資訊見 [npm-advisories.json](test-gap-evidence/npm-advisories.json)。

2026-10-15 再查 registry 與 advisory，braces 最新 3.0.3、node-forge 最新 1.4.0，仍未有修補版；Nuxt 4.6.1／Nitro 2.13.4 也已為最新。`npm audit` 的 `fixAvailable` 雖提出 Nuxt 4.1.3，這是降版建議，不是上游已修補的證據，因此未執行 `npm audit fix --force`。追蹤條件：上述兩個 GHSA 出現 patched version 後，更新 lockfile 並跑 typecheck／test:coverage／build／E2E，直到 high／critical 清零才結束。每週一 UTC 01:00 只執行 `web-audit` 重查 advisory 並保存 JSON，不啟動 DB 或其他品質 jobs。

GitHub `main` 在本次查核時沒有 branch protection 或 ruleset，`web-audit` 並非必要檢查，未修改遠端規則。未來建立 PR 規則時，上游修補前只將 api／web／native／e2e 列為必要檢查；`web-audit` 保持獨立失敗與 artifact。修補完成後再評估將 audit 納入必要檢查。

API 鎖檔的 RSA `RUSTSEC-2023-0071` 屬未啟用的 SQLx MySQL optional dependency。`audit-api.sh` 先檢查所有平台的實際依賴樹，只有 RSA 不在樹中才略過該 advisory；一旦啟用 RSA 會恢復完整 audit 並失敗。原生 audit 無 vulnerability，仍有上游 informational warnings。

API／native 的 cargo-audit 固定 0.22.2，advisory DB 仍每次更新，避免漏掉新公告。本次重新稽核實際發現 rustls 0.23.45 的四個 10/14 新 advisory（RUSTSEC-2026-0346～0349）；API lockfile 已只升級此套件至 [修補版 0.23.46](https://github.com/rustls/rustls/releases/tag/v/0.23.46)，沒有忽略新 advisory。

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

`test_helpers.rs` 的 3 個測試涵蓋 fixture 選項、password 真實 unlock、三種有效認領狀態與 cancelled／expired 排除、三種對帳漂移的拒絕、JSON／空／純文字回應和 malformed JSON 拒絕。`wait_until` 以條件與逾時上限運作；本輪依稽核建議加入 5ms 輪詢間隔。秘密客 privacy fixture 與 quantity fixture 也經單一 wishlist／item 實作，needed／qty 明確指定。

唯一完整財務對帳的例外是 `contribution_visibility_migration_preserves_existing_scope`：它只套用 0001–0005 後驗證 0006 backfill，刻意未扣錢包／累加品項；保留原本 before／after scope assertions，並在測試內註明。舊 schema 尚無 closes_at，因此只在此案例用最小 inline SQL 建清單。

追加的 [mutation 記錄](test-gap-evidence/helper-mutations.json) 與 5 份 raw logs 均為實際跑測試所得，mutant 執行後以 SHA-256 確認原始碼還原：

- 停用認領、錢包、認捐品項任一對帳斷言，負 fixture 測試都會失敗，避免 helper 變成 no-op 還全綠。
- 移除 pledge 的錢包 FOR UPDATE：凍結錢包最終 balance 變成 800，而預期為 1000；由最終 DB 狀態殺死 mutant。
- 改成來源品項優先鎖：預期的 DB blocked statement 不再成立，測試在診斷上限失敗。此追加 log 驗證鎖順序；真實死鎖的資料庫證據仍見上一輪 `wishlist-mutation-delete-deadlock-evidence.txt`，不把本次的等待失敗宣稱為新死鎖證據。

還原後完整 API coverage gate：203 passed、0 failed、0 ignored；lines 91.289%、functions 88.255%、regions 87.157%，覆蓋的行／函式／region 數與原基準完全相同，沒有調低門檻。原始結果為 [完整測試與 coverage log](test-gap-evidence/helper-migration-final.txt)；Rust fmt、clippy `-D warnings` 與 diff whitespace 檢查也通過。上一輪的 200-test artifacts 保持原樣，新增證據區分本次 203-test 結果。

## 稽核後修正與獨立重跑

當輪可重播的程式碼 snapshot 為 `be15ed2`。五個 CDN 站點、十個代表性 CHECK、ledger UPDATE／DELETE、三種對帳與錢包鎖共 21 個 case 均完成原版綠 → 預期語義失敗 → 原始碼還原且重測綠。runner 用 testcase 專屬 panic 訊息判定；編譯錯誤、HTTP 500／連線錯誤、一般 assertion 或單純 lock timeout 都不算 killed。反向轉投的歷史等待失敗證據保留，但不列入當輪 runner 的語義 kill 數；新增案例見下。

`scripts/mutate.sh --list` 顯示名稱；`all` 依序跑所有 case。log、patch、receipt 預設存於 `apps/api/target/mutations/<name>/`，可用 MUTATION_OUTPUT_DIR 指定目錄。exit 0 表示 killed 且還原測試綠；survived／invalid 回非零。不要同時對同一 SQLx test path 跑一般測試；runner 間有 process lock。Ctrl-C／SIGTERM 會先終止 Cargo 子程序群，再還原 source、記錄 interrupted；重跑不沿用舊 killed receipt。三個純 stdlib 測試驗證語義分類、infra 拒絕與中斷還原。

```sh
export DATABASE_URL=postgres://wishsync:wishsync@localhost:5432/wishsync
export APP_ENV=dev
scripts/mutate.sh cdn-account-delete
scripts/mutate.sh cdn-close-due
scripts/mutate.sh check-qty-range
scripts/mutate.sh ledger-update
scripts/mutate.sh ledger-delete
python3 scripts/test_mutate.py
```

稽核需要的原始輸出與精確 patch 皆在 [audit-followup](test-gap-evidence/audit-followup/)。receipt 的 HEAD 是跑測當時的提交位置；上方 replay snapshot 包含當時已測的工作樹變更。修正後 API 205／205，coverage 91.289% lines／88.255% functions／87.157% regions，門檻未調低；fmt／clippy 通過。版本不一致拒絕的負測試也已保存。

已核對有效品項的 qty_claimed 統計，清單／dashboard／SSE／public／profile 完成度均排除 deleted_at；admin stats／orders 不以該欄位計算有效庫存。profiles 的 delivered 捐助歷史直接取 claims.qty，與 `731adb1` 一致，不是保留 qty_claimed 帶來的回歸。wishlist_events.rs 註明 BUS 只綁第一個 pool，同 binary 的未來 SSE 測試需共用 pool。

本輪 npm audit 仍是 11 high／0 critical，保留獨立安全訊號；此次沒有新增 frontend component 覆蓋範圍。toolchain pins 依 [rust-toolchain action 文件](https://github.com/dtolnay/rust-toolchain) 與 [install-action 的版本語法](https://github.com/taiki-e/install-action#example-workflow) 設定。

## 殘留風險補強（2026-10-15）

runner 新增五個併發防護，registry 共 26 案；CI 在 coverage 後依序執行 `scripts/mutate.sh all`，任何 survived／invalid 都失敗並保存 artifact。沒有新增固定 sleep；NOWAIT／blocked statements 確認衝突點，deadlock counter 以條件輪詢等待 PostgreSQL 發布統計。

| 名稱 | mutation 與精確失敗證據 |
| --- | --- |
| `claim-wishlist-for-share` | 移除 claim 的 wishlist FOR SHARE；清單已 archived，最終 DB qty_claimed 卻變 2。 |
| `idempotency-claim-on-conflict` | 移除 ON CONFLICT；兩請求確實重疊、最終一筆 claim，已提交 reservation 的 begin probe 得到 SQLSTATE 23505／idempotency_keys_pkey。 |
| `idempotency-pledge-on-conflict` | 同上，最終一筆 contribution／一次扣點，probe 證實精確 DB 錯誤；不以一般 HTTP 500 作 kill。 |
| `delete-item-for-update` | 移除 item FOR UPDATE；先確認兩種 victim 結果均完整 rollback／reconcile，再觀察 per-DB deadlocks 0→1。 |
| `reallocation-source-first` | 改成來源先鎖；先確認錢包 800／兩品項 pledged 200／兩 contributions／0 orders／reconcile，再觀察 deadlocks 0→1。 |

反向轉投仍不是兩筆可同時合法的業務交換；此測試證實在 eligibility validation 前鎖定品項時，來源優先會造成真實交叉等待。所有 kill 必須有該 case 的專屬語義斷言，infra／單純 timeout 一律 invalid。

```sh
scripts/mutate.sh claim-wishlist-for-share
scripts/mutate.sh idempotency-claim-on-conflict
scripts/mutate.sh idempotency-pledge-on-conflict
scripts/mutate.sh delete-item-for-update
scripts/mutate.sh reallocation-source-first
scripts/mutate.sh all
python3 scripts/test_mutate.py
python3 scripts/test_coverage.py
```

本輪可重播的程式碼提交為 `7234d2f`。從該提交完整重跑 26 案，全部 baseline exit 0 → mutant exit 101 → restored exit 0；receipt 的 HEAD 與原始／還原／提交內容 SHA 全數吻合。完整 [receipts](test-gap-evidence/residual-risk/mutations.json)、每案三份 log 與 patch 均保存在 [residual-risk](test-gap-evidence/residual-risk/)。

最終 API 205／205、0 ignored，覆蓋率仍為 lines 91.289157%／functions 88.255034%／regions 87.156589%，counts 與原基準完全相同；新餘裕沒有用來掩蓋本輪覆蓋率下降。fmt／clippy 通過，runner unit 4／4、coverage policy unit 2／2；併發測試另以預設執行緒重跑五次，85／85 通過。API／native audit exit 0；實際新增 rustls advisory 的失敗與修補後輸出、advisory DB commit、npm 11 high／0 critical、版本與 GitHub PR 規則查核均另存同一證據目錄。


## 後續建議核對與行為回歸

本輪程式碼提交為 `e6e83e8`。P1 五個併發 mutant 已於 `7234d2f` 納入，registry 是 26 案。`wait_until` 的 5ms 輪詢間隔、SSE 首個 pool 的註解及 API 覆蓋率 0.5pp 餘裕也已存在。README 再補明刪除已覆蓋程式碼可能讓比例低於門檻，需重測並同步 baseline。

`deleted_item_stats.rs` 新增實際 HTTP 回歸：先建立 delivered qty=2 品項及 live qty=1／needed=4 正向控制，force delete 後核對 soft delete、保留 delivered claim／cached qty=2、所有 DB 不變式；dashboard 的有效品項／數量／完成率及 /me/export 都只含 live 品項。移除兩個 deleted_at 過濾條件各自造成明確 HTTP 數值斷言失敗；這兩案未加入標準 registry，另有重跑腳本。

`/admin/stats` 沒有 qty_claimed 庫存加總，claims_by_status 是歷史筆數；`731adb1` 的查詢已如此，沒有因保留 delivered cache 而增加數量。本輪保留歷史口徑，測試核對 delivered=1／reserved=1、users／wishlists／active_wishlists 不變。若希望改成只計未刪品項，需明確變更這個統計口徑。

新增 ReportSheet 10、ItemCard 12、GuestTop 1 個實際 DOM 元件測試。ReportSheet 使用後端真實的 403／FORBIDDEN ApiError 測 token 重設／輸入保留／重試；驗證未完成、pending、成功／rate limit、驗證過期／失敗與取消均涵蓋。快速同 tick 雙擊發現重複 HTTP，補一行 busy guard。驗證略過與 pending 提前清除的兩個 frontend mutant 也會被精確呼叫次數斷言抓到。

前端 120／120，Node 22 typecheck 通過；全域覆蓋率 lines 62.92%、statements 60.56%、functions 59.83%、branches 59.81%。ReportSheet 分支 92.30%、ItemCard 分支 87.83%、GuestTop 全項 100%；未宣稱 Turnstile 所有載入生命週期及 ItemCard 所有備援顯示分支均已測。既有前端基準未調低。API 206／206、0 ignored，fmt／clippy／coverage gate 通過，API counts 與原基準仍完全相同。

已建立公開上游追蹤 [issue #7](https://github.com/erictheng93/wishsync/issues/7)，保存 issue readback。npm 仍有 11 high／0 critical；上游修補前 web-audit 保持非 required，CI 每週重查。完整行為與 mutation 證據在 [behavior-followup](test-gap-evidence/behavior-followup/)。

從 `e6e83e8` 再跑 `scripts/mutate.sh all`：26／26 killed，每案原始／還原／提交 source SHA 一致。兩個額外 deleted filter mutant 及兩個 ReportSheet mutant 亦有完整三階段 logs／patch／SHA；後者保存 real 403 fixture 的 test SHA，避免把修改前的錯誤 mock 當作本輪證據。額外 Rust 案例可用 `python3 docs/qa/test-gap-evidence/behavior-followup/deleted-stats/reproduce.py` 重跑（同樣需要 DATABASE_URL／APP_ENV）。
