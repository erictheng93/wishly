# 金流後台 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 新增 `/admin/finance`（總覽＋四個分頁），把金流、付費用戶、渠道、風險與負債整合到同一處。

**Architecture:** 後端新增唯讀模組 `finance.rs`，每個分頁一支 `Staff` 端點，直接查既有資料表，不做預先彙總；交易明細沿用並擴充 `GET /admin/topups`。前端 `finance.vue` 負責分頁與時間區間，各分頁一個元件，分頁定義放陣列以利日後依角色過濾。

**Tech Stack:** Rust（axum、sqlx）、PostgreSQL 16、Nuxt 4、vitest、Playwright。

**Spec:** `docs/superpowers/specs/2026-10-31-finance-dashboard-design.md`，指標定義以 spec 的表格為準。

## Global Constraints

- 所有端點唯讀、`Cache-Control: private, no-store`、使用 `admin::Staff` 守衛，不新增角色、權限表或新型別。
- 時區一律 `Asia/Taipei`；區間是含今日的 N 個日曆天。
- `days` 只接受 1、7、30、90、365，其他回 422（`AppError::invalid("/days", "OUT_OF_RANGE", ...)`）。
- 人工發放（adjustment）絕不計入實收；金額只從 `topups` 算。
- 統計不排除營運人員、demo、已刪除用戶。
- 用戶 email 只出現在 `/admin/finance/payers` 與 `/admin/topups`。
- 文字走 i18n，`zh-Hant`、`en`、`id`、`vi` 四語系的 `admin.json` 都要補。
- 程式註解用繁體中文。
- commit 訊息不加 Co-Authored-By。

## Review Focus

- 空資料庫：所有比率回 `null`，不可除以 0；前端顯示 `–`。→ Task 2、3、5
- 台北午夜：UTC 15:59 與 16:00 的付款落在不同日。→ Task 2
- 爭議勝訴的儲值只算一次實收，且不算拒付。→ Task 2
- 已過期但背景工作還沒改狀態的 `pending` 單不算「等待繳費」。→ Task 2
- 用戶搜尋含 `%`、`_` 不能當萬用字元；已刪除用戶（email 為 NULL）仍查得到交易。→ Task 4

---

### Task 1: 拒付時間與索引

**Files:**
- Create: `db/migrations/0047_finance_dashboard.sql`
- Modify: `apps/api/src/wallet_ops.rs:360,378,407`
- Test: `apps/api/tests/wallet_ops.rs`

**Interfaces:**
- Produces: `topups.updated_at` 反映爭議狀態變更時間；`topups_paid_at_idx`。

- [ ] **Step 1: 寫失敗測試 `dispute_transitions_touch_updated_at`**

建 paid 儲值，把 `updated_at` 改成一天前，依序走 open、won，再走 open、lost，每步斷言 `updated_at > now() - interval '1 minute'`。

- [ ] **Step 2: 執行確認失敗**

Run: `cargo test --manifest-path apps/api/Cargo.toml --test wallet_ops dispute_transitions_touch_updated_at`
Expected: FAIL

- [ ] **Step 3: 三處 `UPDATE topups` 補 `updated_at = now()`，新增 migration**

```sql
CREATE INDEX topups_paid_at_idx ON topups (paid_at) WHERE paid_at IS NOT NULL;
-- 舊拒付的 updated_at 不是拒付時間：以稽核紀錄回填，否則會算到錯的區間
UPDATE topups t SET updated_at = a.at
  FROM (SELECT entity_id, max(created_at) AS at FROM audit_logs
         WHERE action = 'topup.charged_back' AND entity = 'topups' GROUP BY 1) a
 WHERE t.id = a.entity_id AND t.status = 'charged_back';
```

先確認 `wallet_ops.rs:457` 的稽核列 `entity` 是 `topups`，不是的話照實調整條件。

- [ ] **Step 4: 執行確認通過**

Run: `cargo test --manifest-path apps/api/Cargo.toml --test wallet_ops`
Expected: 全數 PASS

- [ ] **Step 5: Commit**

`git commit -m "fix(wallet): 爭議狀態變更時更新 topups.updated_at，並加金流統計索引"`

### Task 2: 總覽 API

**Files:**
- Create: `apps/api/src/finance.rs`
- Modify: `apps/api/src/lib.rs`（`pub mod finance`、`.merge(finance::routes())`）
- Test: `apps/api/tests/finance.rs`

**Interfaces:**
- Consumes: `tests/common` 的 `staff`、`user`、`call`、`ck`、`topup_credit`、`grant`。
- Produces: `finance::routes() -> Router<AppState>`；`fn parse_days(q: &DaysQ) -> R<i32>`；`const RANGE_CTE`（下述 SQL 片段，Task 3 共用）。

共用區間（`$1` 為 days）：

```sql
d0 = (now() AT TIME ZONE 'Asia/Taipei')::date - ($1 - 1)
cur_start = d0::timestamp AT TIME ZONE 'Asia/Taipei'
prev_start = cur_start - make_interval(days => $1)
prev_end = now() - make_interval(days => $1)
```

`GET /admin/finance/overview?days=30` 回傳：

```json
{ "range": {"from": "2026-10-02", "to": "2026-10-31", "days": 30},
  "todo": {"disputed": {"count": 0, "amount": 0}, "cash_refunds": {"count": 0, "amount": 0},
           "debts": {"count": 0, "amount": 0}, "pending": {"count": 0, "amount": 0}},
  "kpis": {"cur": {"gross": 0, "payers": 0, "first_time": 0}, "prev": {"gross": 0, "payers": 0, "first_time": 0}},
  "daily": [{"date": "2026-10-02", "gross": 0}] }
```

`daily` 用 `generate_series` 補零，長度等於 `days`。`todo.debts.amount` 是 `amount - recovered_points`（open）；`todo.cash_refunds` 含 `requested`、`processing`；`todo.pending` 需 `expires_at > now()`。

- [ ] **Step 1: 在 `apps/api/tests/finance.rs` 寫失敗測試**

```rust
overview_requires_staff                // 一般用戶 403
overview_rejects_bad_days              // days=2、days=0 回 422
overview_empty_is_zero                 // 全 0，daily 長度 = days
gross_excludes_manual_grant            // grant 500 不計入 gross
gross_keeps_charged_back_and_won       // charged_back 與勝訴的 paid 都只計一次
first_time_vs_repeat                   // 舊付款在區間外 → 計入 payers，不計 first_time
taipei_midnight_boundary               // 今日台北 00:00 前後一秒分屬昨日、今日
prev_period_cuts_at_same_moment        // days=1：昨天此刻之後的付款不計入 prev
pending_ignores_expired                // expires_at 已過的 pending 不計
todo_debts_nets_recovered              // 欠款 1000、recovered_points 400 → amount 600
```

- [ ] **Step 2: 執行確認失敗**

Run: `cargo test --manifest-path apps/api/Cargo.toml --test finance`
Expected: 編譯或 404 失敗

- [ ] **Step 3: 實作 `overview(State, Staff, Query<DaysQ>) -> R<Response>`**

回應標頭比照 `wallet_ops::json_resp`。整份用單一 `jsonb_build_object` 查詢，比照 `admin.rs::analytics`。

- [ ] **Step 4: 執行確認通過**

Run: `cargo test --manifest-path apps/api/Cargo.toml --test finance`
Expected: 全數 PASS

- [ ] **Step 5: Commit**

`git commit -m "feat(admin): 金流總覽 API"`

### Task 3: 付費用戶、渠道、風險 API

**Files:**
- Modify: `apps/api/src/finance.rs`
- Test: `apps/api/tests/finance.rs`

**Interfaces:**
- Consumes: Task 2 的 `parse_days`、`RANGE_CTE`。
- Produces:

```json
// GET /admin/finance/payers?days=
{ "range": {}, "gross": 0, "payers": 0, "first_time": 0, "repeat_rate": null,
  "buckets": [{"key": "lt500", "count": 0, "amount": 0}],
  "top": [{"user": {"id": "", "display_name": "", "email": null}, "amount": 0, "count": 0,
           "first_paid_at": "", "last_paid_at": ""}] }

// GET /admin/finance/channels?days=
{ "range": {}, "channels": [{"method": "card", "gross": 0, "count": 0, "avg": null,
                             "success_rate": null, "payers": 0}] }

// GET /admin/finance/risk?days=
{ "range": {},
  "liability": {"total": 0, "unused": 0, "refundable": 0, "non_refundable": 0, "pledged": 0},
  "refunds": {"count": 0, "amount": 0}, "chargebacks": {"count": 0, "amount": 0}, "loss_rate": null,
  "disputed": [{"id": "", "amount": 0, "method": "card", "paid_at": "", "display_name": ""}],
  "cash_refunds_open": {"count": 0, "amount": 0}, "debts_open": {"count": 0, "amount": 0},
  "frozen_wallets": 0, "invoices_pending": {"count": 0, "amount": 0} }
```

固定值：`buckets` 永遠回五個 key，依序 `lt500`、`500_999`、`1000_2999`、`3000_9999`、`gte10000`。`top` 前 20 名，依金額降冪、`wallet_id` 為次序。`channels` 永遠含 `card`、`cvs_code`、`cvs_barcode`、`atm`；`line_pay`、`apple_pay` 有資料才出現；依 `gross` 降冪。`success_rate` 分母是區間內 `status <> 'pending'` 的訂單，分子是其中 `paid_at IS NOT NULL` 者。`disputed` 上限 50 筆。`invoices_pending` 取 `source_type = 'topup'`、`status = 'pending'`，且只算對應 topup 已付款的列。`loss_rate` = (退款 + 拒付) ÷ 區間實收。

- [ ] **Step 1: 寫失敗測試**

```rust
payers_buckets_and_top_order           // 五個 key 俱全，前 20 名排序正確，超過 20 人只回 20
payers_repeat_counts_history           // 區間內 1 筆、區間外 1 筆 → 算回購
payers_keeps_deleted_user              // 匿名化用戶仍在，email 為 null
channels_success_rate                  // paid 1、failed 1、expired 1、pending 1 → 1/3
channels_hide_unused_wallet_methods    // 無 line_pay 資料時不回傳
channels_empty_rates_null              // 無訂單時 avg、success_rate 為 null
risk_liability_splits                  // topup 100、adjustment 50、pledged 30 → total 180
risk_no_email_leak                     // 回應字串不含用戶 email；overview、channels 同樣斷言
risk_refund_and_chargeback_windows     // 區間外完成的退款、拒付不計
risk_loss_rate_null_when_no_gross
```

- [ ] **Step 2: 執行確認失敗**

Run: `cargo test --manifest-path apps/api/Cargo.toml --test finance`
Expected: 新測試 FAIL

- [ ] **Step 3: 實作 `payers`、`channels`、`risk`，比照 `overview`**

- [ ] **Step 4: 執行確認通過**

Run: `cargo test --manifest-path apps/api/Cargo.toml --test finance`
Expected: 全數 PASS

- [ ] **Step 5: Commit**

`git commit -m "feat(admin): 付費用戶、渠道、風險與負債 API"`

### Task 4: 交易明細篩選

**Files:**
- Modify: `apps/api/src/wallet_ops.rs:466-496`
- Test: `apps/api/tests/wallet_ops.rs`

**Interfaces:**
- Produces: `GET /admin/topups` 新增查詢參數 `method`、`q`、`user_id`，回應不變。

`PageQ` 是多處共用，在 `wallet_ops.rs` 另定 `TopupQ { cursor, limit, status, method, q, user_id }`。`q` 比對 `u.display_name`、`u.email`，不分大小寫，用 `position(lower($q) IN lower(...)) > 0`，避免 `LIKE` 萬用字元。`method` 驗證規則比照 `status`，錯誤碼 `ENUM`。

- [ ] **Step 1: 寫失敗測試**

```rust
topups_filter_by_method_and_user       // 各自只回對應列
topups_q_is_literal                    // q="%" 不命中無百分號的用戶
topups_q_cursor_paginates              // limit=1 走 cursor 能取完全部命中列
topups_deleted_user_still_listed       // email 為 null，依 user_id 查得到
topups_bad_method_422
```

- [ ] **Step 2: 執行確認失敗**

Run: `cargo test --manifest-path apps/api/Cargo.toml --test wallet_ops topups_`
Expected: FAIL

- [ ] **Step 3: 改寫 `topup_list`**

- [ ] **Step 4: 執行確認通過**

Run: `cargo test --manifest-path apps/api/Cargo.toml --test wallet_ops`
Expected: 全數 PASS

- [ ] **Step 5: Commit**

`git commit -m "feat(admin): 儲值列表支援渠道、用戶篩選"`

### Task 5: 前端外殼與總覽分頁

**Files:**
- Create: `apps/web/app/pages/admin/finance.vue`、`apps/web/app/components/finance/Overview.vue`、`apps/web/app/utils/finance.ts`、`apps/web/tests/finance.test.ts`
- Modify: `apps/web/app/pages/admin/analytics.vue`（共用的 `.an-*` 樣式搬到 `console.css`）、`apps/web/app/assets/console.css`、`apps/web/app/pages/admin/index.vue`（導覽加入口）、四語系 `admin.json`

**Interfaces:**
- Consumes: Task 2 的 overview 回應；既有 `fmtPts`、`fmtCount`、`pct`、`fmtTime`、`useApi`、`errMsg`。
- Produces（`utils/finance.ts`）：

```ts
export const FINANCE_RANGES = [1, 7, 30, 90, 365] as const
export const FINANCE_TABS = ['overview', 'transactions', 'payers', 'channels', 'risk'] as const
export type FinanceTab = typeof FINANCE_TABS[number]
export const financeTab = (q: unknown): FinanceTab  // 未知值回 'overview'
export const delta = (cur: number, prev: number): number | null  // prev 為 0 回 null
export const perPayer = (gross: number, payers: number): number | null  // payers 為 0 回 null
```

分頁元件收 `days` prop，自行呼叫 API，並在 `days` 變動時重載。`finance.vue` 以 `<component>` 或 `v-if` 切換，分頁存 `route.query.tab`，用 `router.replace` 更新。時間區間按鈕沿用 `.an-range` 樣式。`transactions` 分頁不顯示區間按鈕。

待處理四格的連結：爭議中 → `?tab=transactions&status=disputed`、退款 → `/admin/cash-refunds`、欠款 → `/admin/wallet-debts`、等待繳費 → `?tab=transactions&status=pending`。核心數字以 `title` 與可見的一句說明顯示定義，文字取自 spec 指標表。

- [ ] **Step 1: 在 `tests/finance.test.ts` 寫失敗測試**

```ts
financeTab('x') === 'overview'; financeTab('risk') === 'risk'
delta(120, 100) === 20; delta(5, 0) === null; delta(0, 100) === -100
perPayer(1000, 0) === null; perPayer(1000, 4) === 250
```

- [ ] **Step 2: 執行確認失敗**

Run: `cd apps/web && npx vitest run tests/finance.test.ts`
Expected: FAIL

- [ ] **Step 3: 實作 `utils/finance.ts`、`finance.vue`、`Overview.vue`，補 i18n**

每個分頁都要有載入中、空資料、錯誤加重試狀態。

- [ ] **Step 4: 執行驗證**

Run: `cd apps/web && npm run typecheck && npm test`
Expected: 全數通過，含 `i18nMessages.test.ts`

- [ ] **Step 5: Commit**

`git commit -m "feat(admin): 金流後台外殼與總覽分頁"`

### Task 6: 付費用戶、渠道、風險分頁

**Files:**
- Create: `apps/web/app/components/finance/Payers.vue`、`Channels.vue`、`Risk.vue`
- Modify: `apps/web/app/pages/admin/finance.vue`、四語系 `admin.json`

**Interfaces:**
- Consumes: Task 3 的回應；Task 5 的 `perPayer`。

級距用 `.an-row` 長條；前 20 名與渠道比較用表格，手機版可橫向捲動。前 20 名點擊導向 `?tab=transactions&user_id=<id>`。比率為 `null` 時顯示 `–`。風險分頁的「待開立發票」旁加一句說明：電子發票尚未串接。退款、欠款連到既有頁面。

- [ ] **Step 1: 實作三個元件並接進 `finance.vue`**

- [ ] **Step 2: 執行驗證**

Run: `cd apps/web && npm run typecheck && npm test`
Expected: 全數通過

- [ ] **Step 3: Commit**

`git commit -m "feat(admin): 付費用戶、渠道、風險分頁"`

### Task 7: 交易明細搬移與收尾

**Files:**
- Create: `apps/web/app/components/finance/Transactions.vue`、`apps/web/e2e/admin-finance.spec.ts`
- Modify: `apps/web/app/pages/admin/topups.vue`、`apps/web/app/pages/admin/index.vue:145`、`apps/web/e2e/admin-wallet-ops.spec.ts`、`apps/web/app/assets/console.css:66`、四語系 `admin.json`、`docs/04-data-schema-api-contract.html`、`CLAUDE.md`、`README.md`（若提及舊路徑）

**Interfaces:**
- Consumes: Task 4 的查詢參數。`status`、`user_id` 初值取自 `route.query`。

把 `topups.vue` 的列表、爭議操作、確認對話框完整搬到 `Transactions.vue`，爭議的 `Idempotency-Key` 行為不變。`status` 預設改為空字串（全部）。新增渠道下拉與用戶搜尋。`topups.vue` 只留導向：`navigateTo('/admin/finance?tab=transactions', { replace: true })`，保留 `middleware: 'staff'`。首頁「儲值與爭議」按鈕移除，改由 Task 5 的金流後台入口取代。`console.css` 的 `.k-kpis dt` 由 10.5px 改 11px。

`CLAUDE.md` 架構段落加一行：金流後台在 `finance.rs`，指標定義見 spec。

- [ ] **Step 1: 寫 e2e `admin-finance.spec.ts`**

以 `page.route` 模擬 API，比照 `admin-wallet-ops.spec.ts`：

```ts
test('總覽顯示待處理與核心數字，切換區間會帶 days 重查')
test('前 20 名點擊後，交易明細帶 user_id 篩選')
test('/admin/topups 導向交易明細分頁')
test('API 失敗時顯示錯誤與重試')
```

- [ ] **Step 2: 搬移實作，並把 `admin-wallet-ops.spec.ts` 的爭議流程改走新路徑**

- [ ] **Step 3: 執行全套驗證**

Run: `cd apps/web && npm run typecheck && npm test && npm run build && npx playwright test admin-finance admin-wallet-ops`
Expected: 全數通過

Run: `cargo test --manifest-path apps/api/Cargo.toml`
Expected: 全數通過

- [ ] **Step 4: 實機確認**

以 `docker compose` 與 dev server 開 `/admin/finance`，逐一檢查五個分頁在淺色、深色、手機寬度下的顯示，並確認無 console 錯誤。

- [ ] **Step 5: Commit**

`git commit -m "feat(admin): 交易明細併入金流後台，舊路徑導向"`
