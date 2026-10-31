# 金流後台設計

日期：2026-10-31　狀態：待使用者確認

## 目的

Wishly 的金流資料目前散在 `/admin/topups`、`/admin/wallets`、`/admin/cash-refunds`、`/admin/wallet-debts` 四頁，全部是逐筆清單，沒有任何加總或趨勢。`/admin` 首頁與 `/admin/analytics` 也完全沒有金額。

新增一個金流後台 `/admin/finance`，讓平台營運團隊在同一處看到金流全貌，並據此做決策。要回答的問題：錢何時進來、有多少、從哪個渠道來、是誰儲值的、付費客戶有多少、付費用戶是什麼樣子、各渠道表現如何。

使用者是財務與客服，各看各的分頁，所以採「總覽＋分頁」版面。

## 目標

- 一個頁面看到金流、付費用戶、渠道、風險與負債。
- 待處理事項（爭議、退款、欠款）打開就看得到，並能一鍵前往處理。
- 區間總額能和綠界後台對得起來。`paid_at` 是收到通知的時間，不是綠界的付款時間，所以跨日邊界附近的單筆可能差一天，差異要能用交易明細解釋；不做逐日完全一致。
- 之後要分財務、客服權限時，不必重寫頁面或 API。

## 非目標

- 行銷成效。系統沒有來源資料（無 UTM、推薦碼、活動代碼），使用者已決定先不處理。
- 角色權限。專案還小，所有營運人員都看得到全部內容。
- 匯出 CSV、自訂日期區間、圖表下鑽。
- 預先彙總表或快取。資料量小，直接即時查詢。
- 把現金退款、欠款的處理流程搬進新頁面，這兩頁維持原樣。

## 反目標

做到以下任一項都算失敗，即使功能跑得起來：

- 儀表板數字與綠界後台對不上，且無法解釋差異。
- 把人工發放的點數算進實收。
- 在 `/public/*` 或任何可被 CDN 快取的路徑露出金額或用戶資料。

## 版面

路由 `/admin/finance`，分頁狀態放在網址 `?tab=`，可以直接分享連結。沿用紅白實驗室主題與 `console.css`（`.c-admin`、`.k-kpis`），分頁用既有 `.c-tabs`，圖表沿用 `analytics.vue` 的 SVG 折線與 `.an-row` 長條寫法。`/admin` 首頁的金流導覽區加入口。

時間區間：今日、7、30、90、365 天，預設 30 天，跨分頁共用。

### 總覽（`overview`）

1. 待處理：爭議中、待處理退款、未結欠款、等待繳費，各顯示筆數與金額，點擊前往對應處理處。不隨時間區間變動。
2. 核心數字：儲值實收、付費用戶、首次付費、平均每人，各附與前期比較。
3. 每日實收折線。

### 交易明細（`transactions`）

取代現有 `/admin/topups`。逐筆列出時間、用戶（名稱與 email）、渠道、金額、狀態，既有的拒付處理按鈕與確認對話框整組搬過來。

篩選：狀態、渠道、用戶（名稱或 email 關鍵字）、指定 `user_id`。預設顯示全部狀態，不再預設爭議中。此分頁不套用共用的時間區間，顯示最新交易並往下載入。

`/admin/topups` 導向 `/admin/finance?tab=transactions`。

### 付費用戶（`payers`）

- 數字：付費用戶、首次付費、回購率、平均每人。
- 金額級距分布：`< 500`、`500–999`、`1,000–2,999`、`3,000–9,999`、`≥ 10,000`，以區間內每位用戶的累計儲值分組。
- 儲值金額前 20 名：用戶、累計金額、筆數、首次與最後付款時間。點擊導向交易明細並帶 `user_id`。

### 渠道（`channels`）

依付款方式（`card`、`cvs_code`、`cvs_barcode`、`atm`）比較實收、筆數、平均金額、付款成功率、付費人數。`line_pay`、`apple_pay` 雖在 enum 內但目前未開放，有資料才顯示。

### 風險與負債（`risk`）

- 點數負債 = 未使用點數 + 認捐中。未使用點數再拆成可退現金（`topup` 批次）與不可退現金（`gift_card`、`reward_bonus`、`adjustment`）。認捐時點數已從餘額扣除，所以兩者相加不重複。
- 退款加拒付佔實收的比例，以及區間內各自的金額。
- 爭議中清單。
- 待處理現金退款、未結欠款的筆數與金額，連到既有頁面。
- 凍結錢包數。
- 待開立發票的筆數與金額。電子發票尚未串接，invoices 列會一直停在 pending（`topup.rs:249` 的 shortcut），這個數字會持續累積，頁面要照實顯示。

## 指標定義

全部金額以 NT$ 計（1 點 = NT$1），時間以 `Asia/Taipei` 計。區間是含今日的 N 個日曆天。

| 指標 | 定義 |
|---|---|
| 儲值實收 | `paid_at` 在區間內的 topups 金額加總，含後來變成 `disputed`、`charged_back` 的，等同當時實際進帳，可與綠界對帳 |
| 付費用戶 | 區間內有上述儲值的不重複錢包數 |
| 首次付費 | 付費用戶中，該錢包有史以來第一筆 `paid_at` 落在區間內者 |
| 平均每人 | 儲值實收 ÷ 付費用戶 |
| 回購率 | 區間內付費用戶中，歷史累計付款 2 次以上者的比例 |
| 付款成功率 | `created_at` 在區間內且已結案的訂單中，曾付款者的比例。曾付款：`paid_at` 非空；已結案：排除 `pending` |
| 等待繳費 | 目前 `status = 'pending'` 且未過期的訂單 |
| 退款 | `completed_at` 在區間內的 `cash_refund_requests` 金額加總 |
| 拒付 | `status = 'charged_back'` 且 `updated_at` 在區間內的 topups 金額加總 |
| 未使用點數 | `point_wallets.balance` 加總，分類取自 `point_lots.remaining_points` |
| 認捐中 | `contributions` 狀態為 `pledged` 的點數加總。`captured` 已達標採購、點數已花出，不計入 |

與前期比較：前期是往前平移 N 天的同長度區間，結束時間為「現在減 N 天」，所以看「今日」時比的是昨天同一時刻以前，不會拿未過完的今天比完整的昨天。前期為 0 時不顯示百分比。

統計不排除營運人員或 demo 帳號：這些帳號若真的付過錢，綠界也會有紀錄，排除了就對不上帳。已刪除的用戶同樣保留：帳號刪除只匿名化（`display_name` 變成「已刪除的使用者」、`email` 為 NULL），users 列仍在，交易與統計都照算，email 顯示為空。`refunded` 狀態目前沒有任何程式會設定，不特別處理。

## 後端

新增 `apps/api/src/finance.rs`，提供 `routes()` 並在 `lib.rs::app()` 合併。全部為唯讀 GET，回應帶 `private, no-store`，使用 `Staff` 守衛（含 MFA 檢查）。

| 端點 | 內容 |
|---|---|
| `GET /admin/finance/overview?days=` | 待處理、核心數字、每日實收 |
| `GET /admin/finance/payers?days=` | 付費用戶數字、級距、前 20 名（含 email） |
| `GET /admin/finance/channels?days=` | 各渠道比較 |
| `GET /admin/finance/risk?days=` | 負債、退款與拒付、爭議、待處理、凍結錢包、待開發票 |

`days` 允許 1、7、30、90、365，其他值回 422。既有分析 API 的下限是 7，這裡獨立驗證，不共用。

交易明細沿用 `GET /admin/topups`，加 `method`、`q`、`user_id` 查詢參數；用 `q` 查詢用戶時仍維持 cursor 分頁。

Migration `0047`：`CREATE INDEX topups_paid_at_idx ON topups (paid_at) WHERE paid_at IS NOT NULL`。

修正：`wallet_ops.rs` 的 `dispute_open`、`dispute_won`、`charge_back` 三個 `UPDATE topups` 沒有更新 `updated_at`，且 `topups` 沒有 trigger，導致拒付時間無從計算。三處都補上 `updated_at = now()`。Migration `0047` 同時回填既有 `charged_back` 列：`updated_at` 改用 `audit_logs` 內該筆拒付紀錄的時間，否則舊拒付會算到錯的區間。

## 權限擴充點

這次不做角色，只把界線留在正確的位置，不新增任何權限表、型別或設定：

- 每個分頁一支端點，所以日後角色可以逐端點換守衛，例如把 `Staff` 換成 `Finance`。
- 用戶 email 只出現在 `transactions`、`payers` 兩處，`overview`、`channels`、`risk` 不回傳用戶個資（`risk` 的爭議中清單只回顯示名稱）。之後要限制個資，只需要處理這兩處。
- 前端分頁定義成一個陣列，不把分頁寫死在範本裡，日後依角色過濾這個陣列即可。

## 前端

- `apps/web/app/pages/admin/finance.vue`：外殼、分頁、時間區間。`middleware: 'staff'`，`/admin/**` 已在 `routeRules` 設為 `ssr: false`。
- 各分頁一個元件，放 `apps/web/app/components/finance/`。進入該分頁才載入資料，切換時間區間只重載目前分頁。
- 交易明細元件由現有 `topups.vue` 搬移，沿用 `utils/walletOps.ts`。
- 指標計算與格式化放 `utils/`，比較百分比與前期為 0 的處理要有 vitest。
- 文字走 i18n，四語系（`zh-Hant`、`en`、`id`、`vi`）的 `admin.json` 都要補。
- 每張核心數字提供一句定義說明，讓財務知道數字怎麼算出來的。
- 錯誤沿用 `errMsg`，每個分頁有載入中、空資料、錯誤與重試狀態。

既有 `console.css` 的 `.k-kpis dt` 是 10.5px，低於 11px 的可讀性下限，順手調成 11px。

## 測試

- `apps/api/tests/finance.rs`（`sqlx::test`）：各指標定義都要有案例，至少涵蓋人工發放不計入實收、`charged_back` 仍計入實收且計入拒付、台北時區跨日邊界、前期區間切點、首次付費與回購、非營運人員回 403。
- `wallet_ops` 既有測試補 `updated_at` 的斷言。
- `apps/web/e2e/admin-wallet-ops.spec.ts` 的爭議流程改走新路徑，並驗證 `/admin/topups` 會導向。
- 新增 SQL 皆為唯讀，不涉及鎖序，不需要新增 mutant。

## 文件

- `docs/04-data-schema-api-contract.html`：補上新端點與 `topups` 查詢參數。
- `CLAUDE.md`：在架構段落加一行描述 `finance.rs` 與指標定義的位置。

## 留給實作者決定

- 查詢要一支大 SQL 還是分幾支。
- 元件內部切分、圖表細節、手機版排版。
- 級距與前 20 名的 SQL 寫法。
