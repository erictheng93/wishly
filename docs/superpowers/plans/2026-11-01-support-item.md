# 支持品項 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 讓使用者把別人的公開清單放進自己的清單當「支持品項」，粉絲點入並完成捐助後，累計數字公開顯示在該品項上。

**Architecture:** 新增 `funding_mode = 'support'`，現有 claims / contributions 的條件式 UPDATE 會自動拒絕對它的捐助。捐助列帶無外鍵的 `via_item_id`，成果在讀取時彙總。新模組 `support.rs` 集中目標驗證、彙總與通知；前端用 localStorage 保留 7 天歸因。

**Tech Stack:** Rust（axum + sqlx）、PostgreSQL 16、Nuxt 4、Vitest。

**Spec:** `docs/superpowers/specs/2026-11-01-support-item-design.md`

## Global Constraints

- 文件與註解用繁體中文；測試用 `#[sqlx::test(migrations = "../../db/migrations")]`。
- 任何回應、通知都不可帶捐助者身分，只有彙總數字。
- 目標不可公開提供時，回應不得含對方名稱、封面、slug。
- 歸因失敗不可讓捐助失敗；歸因不成立時 `via_item_id` 寫 `NULL`。
- 歸因保留 7 天，同一對方清單只留最後一次點入。
- 支持品項建立限額：每人每小時 20 次。
- `wishlist.supported`：Social、InAppOnly，四語文案。
- 含支持品項的公開清單：`public, s-maxage=10`、無 ETag。
- 提交前跑 `cargo fmt --check` 與 `cargo clippy --all-targets -- -D warnings`。
- commit 不加 `Co-Authored-By`，也不加 Claude 署名。
- migration 從 0056 開始：`codex/admin-ops-followups` 分支已占用 0055。

## Review Focus

1. 只含支持品項的清單：完成度不可顯示 100% 或除以零，`completion_pct` 為 0，`fulfilled_count` 不把 0/0 算成完成（Task 1 測試）。
2. 對方清單在粉絲點入後才下架或刪除：歸因驗證只看品項是否仍指向該清單，捐助流程照既有規則處理（Task 3 測試）。
3. 粉絲從登入頁回來，或儲值後才回到分享頁：沒有 `via` 參數時仍要讀到 localStorage 的歸因（Task 6 測試）。
4. localStorage 不可用或內容損毀：歸因回 `undefined`，捐助照常（Task 6 測試）。
5. 重放同一個 `Idempotency-Key`：不重複計入（Task 3 測試）。

---

### Task 1: 資料結構與統計排除

**Files:**
- Create: `db/migrations/0056_support_mode.sql`、`db/migrations/0057_support_items.sql`
- Modify: `apps/api/src/wishlists.rs`（`units`、`CLAIMED_SQL`、`OPEN_SQL`、`dashboard`、列表 completion）
- Modify: `apps/api/src/public.rs:100,325,499`、`apps/api/src/profiles.rs:88`、`apps/api/src/friends.rs:396`、`apps/api/src/dashboard.rs:379-384`
- Test: `apps/api/tests/support_items.rs`

**Interfaces:**
- Produces: `wishlist_items.support_wishlist_id`、`support_notified_at`；`contributions.via_item_id`、`claims.via_item_id`；`units("support", ..) == (0, 0)`；`pub const NEEDED_SQL: &str`（`wishlists.rs`，無別名）。

- [ ] **Step 1: 寫失敗測試**

`support_item_excluded_from_completion`：以 SQL 插入 support 品項（`INSERT ... funding_mode, support_wishlist_id`）到含一個 qty_needed=2、qty_claimed=0 品項的公開清單，斷言 `GET /public/wishlists/{slug}` 的 `completion` 為 `{item_count: 1, fulfilled_count: 0, completion_pct: 0}`。

`only_support_items_is_not_complete`：清單只有 support 品項時 `{item_count: 0, fulfilled_count: 0, completion_pct: 0}`。

`relief_needs_ignore_support_item`：救災據點含 support 品項（category、urgency 有值）時，事件頁 `needs` 計數不含它。

- [ ] **Step 2: 執行確認失敗**

Run: `cargo test --manifest-path apps/api/Cargo.toml --test support_items`
Expected: FAIL，欄位或 enum 值不存在。

- [ ] **Step 3: 寫 migration**

`0056_support_mode.sql` 只有 `ALTER TYPE funding_mode ADD VALUE 'support';`。enum 新值在同一交易不能使用，所以另開一檔。

`0057_support_items.sql`：

```sql
ALTER TABLE wishlist_items
  ADD COLUMN support_wishlist_id uuid REFERENCES wishlists(id) ON DELETE SET NULL,
  ADD COLUMN support_notified_at timestamptz,
  ADD CONSTRAINT items_support_shape CHECK (funding_mode = 'support' OR (support_wishlist_id IS NULL AND support_notified_at IS NULL));
-- 刻意不加外鍵：外鍵檢查會對發起者的品項加 KEY SHARE 鎖，把它捲進對方清單的鎖序（同 0050）
ALTER TABLE contributions ADD COLUMN via_item_id uuid;
ALTER TABLE claims ADD COLUMN via_item_id uuid;
CREATE INDEX contributions_via_idx ON contributions (via_item_id) WHERE via_item_id IS NOT NULL;
CREATE INDEX claims_via_idx ON claims (via_item_id) WHERE via_item_id IS NOT NULL;
```

- [ ] **Step 4: 修改統計**

- `units`：`mode == "support"` 回 `(0, 0)`。
- `CLAIMED_SQL`：`WHEN funding_mode = 'support' THEN 0`。
- `OPEN_SQL`：`WHEN funding_mode = 'support' THEN false`。
- 新增 `NEEDED_SQL`：`CASE funding_mode WHEN 'crowdfund' THEN 1 WHEN 'support' THEN 0 ELSE qty_needed END`，取代 `public.rs:100`、`profiles.rs:88`、`friends.rs:396` 的內嵌 CASE。
- `public.rs` 的完成度迴圈、`wishlists.rs::dashboard`、`dashboard.rs:379`：`n == 0` 時 `continue`，`item_count` 只算參與完成度的品項。
- `public.rs:499` 急需統計加 `AND i.funding_mode <> 'support'`。
- `grep -n '"completion"\|funding_mode = .crowdfund. THEN 1' apps/api/src/*.rs` 確認沒有遺漏。

- [ ] **Step 5: 執行測試確認通過**

Run: `cargo test --manifest-path apps/api/Cargo.toml --test support_items`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add db/migrations apps/api/src apps/api/tests/support_items.rs
git commit -m "feat(api): 新增支持品項資料結構並排除於完成度統計"
```

### Task 2: 建立與修改支持品項

**Files:**
- Create: `apps/api/src/support.rs`
- Modify: `apps/api/src/lib.rs`（`pub mod support;`）、`apps/api/src/wishlists.rs`（`item_create`、`apply_funding`、`I`、`ICOLS`、`I::json`）、`apps/api/src/public.rs`（`LIVE_SITE` 改 `pub(crate)` 並移到模組外可引用處）
- Test: `apps/api/tests/support_items.rs`

**Interfaces:**
- Consumes: Task 1 的欄位。
- Produces:
  - `pub const TARGET_LIVE_SQL: &str`：別名 `w`、`o`（owner）的 SQL 條件，即清單可公開提供：`w.deleted_at IS NULL AND w.visibility = 'public' AND w.status = 'active' AND w.moderation_status = 'ok' AND (w.closes_at IS NULL OR w.closes_at > now()) AND o.deleted_at IS NULL AND o.blocked_at IS NULL AND (w.type <> 'relief' OR (LIVE_SITE))`。
  - `pub async fn resolve_target(tx: &mut Tx<'_>, slug: &str, uid: Uuid) -> Result<Uuid, AppError>`：回對方 wishlist id。
  - API：`POST /wishlists/{id}/items` 接受 `funding_mode: "support"` 與 `support_slug: string`。

- [ ] **Step 1: 寫失敗測試**

- `create_support_item_without_date`：A 建品項指向 B 的公開清單，回 201，`funding_mode == "support"`，`funding_deadline` 為 null。
- `create_rejects_unavailable_target`：指向 friends、private、不存在的 slug，都回 422、code `SUPPORT_TARGET_INVALID`，三者 body 的 detail 相同。
- `create_rejects_own_list`：指向自己的清單回 422 `SUPPORT_TARGET_SELF`。
- `create_rejects_manageable_org_list`：組織 coordinator 指向自己組織的據點回 422 `SUPPORT_TARGET_SELF`。
- `create_rate_limited`：第 21 次回 429。
- `support_mode_cannot_change`：PATCH `funding_mode: "crowdfund"` 回 409 `FUNDING_MODE_LOCKED`；PATCH 既有 quantity 品項 `funding_mode: "support"` 回 422。
- `support_target_is_immutable`：PATCH 帶 `support_slug` 回 422。
- `claim_and_pledge_rejected_on_support_item`：對支持品項 POST claims、contributions 都失敗，`qty_claimed`、`pledged_points` 不變。

- [ ] **Step 2: 執行確認失敗**

Run: `cargo test --manifest-path apps/api/Cargo.toml --test support_items create_`
Expected: FAIL，ENUM 錯誤。

- [ ] **Step 3: 實作 `resolve_target` 與 `item_create` 分支**

`resolve_target` 查 `SELECT w.id, can_manage_wishlist(w.owner_id, w.org_id, $2) ... WHERE w.slug = $1 AND {TARGET_LIVE_SQL}`。查無回 `SUPPORT_TARGET_INVALID`（pointer `/support_slug`），可管理回 `SUPPORT_TARGET_SELF`。

`item_create`：`one_of` 加入 `"support"`；support 時先 `ratelimit::check(&mut *tx, &format!("support_item:{}", u.id), 20, 3600)`，呼叫 `resolve_target`，INSERT 帶 `support_wishlist_id`。`INSERT` 增加第 21 個參數。`I` 與 `ICOLS` 增加 `support_wishlist_id`。

- [ ] **Step 4: 實作 `apply_funding` 限制**

`i.funding_mode == "support"` 且 `want` 為其他值時回 409 `FUNDING_MODE_LOCKED`。PATCH 帶 `support_slug` 回 422 `SUPPORT_TARGET_LOCKED`。`one_of` 維持只接受 quantity / crowdfund，所以轉成 support 會回 422。

- [ ] **Step 5: 執行測試確認通過**

Run: `cargo test --manifest-path apps/api/Cargo.toml --test support_items`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add apps/api/src apps/api/tests/support_items.rs
git commit -m "feat(api): 建立支持品項並驗證目標清單"
```

### Task 3: 歸因、彙總與公開回應

**Files:**
- Modify: `apps/api/src/support.rs`、`apps/api/src/claims.rs:119,137`（`CreateReq`、INSERT）、`apps/api/src/contributions.rs:125,257,584`、`apps/api/src/public.rs`、`apps/api/src/wishlists.rs`（`get_one`、`dashboard`）
- Test: `apps/api/tests/support_items.rs`

**Interfaces:**
- Consumes: `TARGET_LIVE_SQL`。
- Produces:
  - `pub async fn valid_via(pool: &PgPool, via: Option<Uuid>, target_wishlist: Uuid) -> Result<Option<Uuid>, AppError>`：不成立回 `Ok(None)`。
  - `pub async fn summaries(pool: &PgPool, item_ids: &[Uuid]) -> Result<HashMap<Uuid, Value>, AppError>`。
  - 品項 JSON 多一個 `support` 物件：`{ "ended": bool, "target": { "slug", "title", "cover_image_url", "cover_preset", "owner_name" } | null, "points": i64 | null, "qty": i64 | null }`。`owner_name` 是組織顯示名稱或個人暱稱。

- [ ] **Step 1: 寫失敗測試**

- `pledge_via_support_item_counts`、`claim_via_support_item_counts`：帶 `via_item_id` 捐助後，公開回應 `support.points`、`support.qty` 正確。
- `via_ignored_when_invalid`：不帶、指向別份清單、已刪除的品項、非 support 品項時，回 201 且不計入。
- `withdraw_and_expiry_reduce_count`：撤回 contribution、部分退款、取消 claim 後數字減少。
- `reallocate_keeps_attribution`：轉投後數字不變。
- `replay_not_counted_twice`：同一 `Idempotency-Key` 重放後數字不變。
- `ended_target_hides_details`：對方改 private、下架、刪除、組織撤銷認證，回應 `ended == true`、`target == null`，數字保留，且整個 body 字串不含對方標題與 slug。
- `via_survives_target_ended_after_click`：對方清單仍可捐助時（closed 例外依既有規則），`valid_via` 只看品項指向，不看 `TARGET_LIVE_SQL`。
- `surprise_target_hides_numbers`：對方是鎖定中的驚喜清單時 `points`、`qty` 為 null，owner 的 `GET /wishlists/{id}` 也是 null。
- `support_list_cache_headers`：含支持品項的公開清單 `cache-control` 為 `public, s-maxage=10`，無 `etag`；帶 `If-None-Match` 仍回 200。無支持品項的清單標頭不變。
- `no_donor_identity_in_response`：`support` 物件只有規定的鍵。

- [ ] **Step 2: 執行確認失敗**

Run: `cargo test --manifest-path apps/api/Cargo.toml --test support_items via`
Expected: FAIL

- [ ] **Step 3: 實作歸因寫入**

兩個 `CreateReq` 加 `via_item_id: Option<Uuid>`。`create` 在開交易前呼叫 `valid_via`（claims 用 `wl` 的 wishlist id，contributions 用 `acl` 的），不加鎖，所以不動既有鎖序。INSERT 增加 `via_item_id`。`contributions.rs:584` 的 reallocate `SELECT` 沿用 `via_item_id`。

`valid_via` 條件：`i.id = $1 AND i.deleted_at IS NULL AND i.funding_mode = 'support' AND i.support_wishlist_id = $2`。

- [ ] **Step 4: 實作 `summaries`**

一條 SQL，對每個 support 品項 LEFT JOIN 對方清單，並彙總：

- 點數：`status IN ('pledged','captured')` 的 `points - refunded_points`。
- 物資：`status IN ('reserved','purchased','delivered')` 的 `qty`。
- `ended`：`NOT COALESCE({TARGET_LIVE_SQL}, false)`。
- `target`：`ended` 時為 `null`。
- 對方 `surprise_locked` 為真時，`points`、`qty` 為 `null`。

- [ ] **Step 5: 接上回應與快取**

`public.rs` 建 `item_json` 前呼叫 `summaries`，非空時：support 品項加 `support`、不輸出 `claimers`，並把未限制路徑的 `cache-control` 設為 `public, s-maxage=10`，跳過 `If-None-Match` 比對和 `ETag`。ETag 比對在讀品項之前，所以先用 `EXISTS` 查清單是否含 support 品項，再決定是否進入 304 分支。

`wishlists.rs::get_one`、`dashboard` 的 items 同樣合併 `support`。

- [ ] **Step 6: 執行測試確認通過**

Run: `cargo test --manifest-path apps/api/Cargo.toml --test support_items`
Expected: PASS

- [ ] **Step 7: 執行既有回歸測試**

Run: `cargo test --manifest-path apps/api/Cargo.toml --test claims --test crowdfund_pledge --test crowdfund_owner --test cdn_invalidation --test concurrency`
Expected: PASS

- [ ] **Step 8: Commit**

```bash
git add apps/api/src apps/api/tests/support_items.rs
git commit -m "feat(api): 歸因捐助並公開支持品項成果"
```

### Task 4: 站內通知

**Files:**
- Modify: `apps/api/src/notify.rs`（`KINDS`、`category`、`email_tier`、`inbox_link`、`render` 四語）、`apps/api/src/support.rs`、`apps/api/src/wishlists.rs`（`item_create`、發佈分支）、`docs/notification-policy.md`
- Test: `apps/api/tests/support_items.rs`

**Interfaces:**
- Produces: `pub async fn notify_targets(tx: &mut Tx<'_>, wishlist_id: Uuid) -> Result<(), AppError>`：通知該清單所有 `support_notified_at IS NULL` 的支持品項，一條語句內完成標記與 INSERT。

- [ ] **Step 1: 寫失敗測試**

- `published_list_notifies_target_owner`：已發佈清單新增品項，對方收到一筆 `wishlist.supported`，payload 含 `wishlist_id`（對方）、`supporter_name`、`list_title`，`inbox_link` 為 `/lists/{wishlist_id}/edit`。
- `draft_notifies_on_publish_once`：草稿不發，發佈後發一筆，再 PATCH 不重發。
- `org_target_notifies_owners_and_coordinators`：volunteer 和已停用成員不收。
- `supported_is_in_app_only`：跑 worker 後站內有內容，Mailpit 無信，`email_tier == InAppOnly`，`category == Social`，並沿用 `notify.rs` 的 KINDS 單元測試。

- [ ] **Step 2: 執行確認失敗**

Run: `cargo test --manifest-path apps/api/Cargo.toml --test support_items notifies`
Expected: FAIL

- [ ] **Step 3: 實作**

`category`：`"wishlist.supported"` 明確回 `Category::Social`。`email_tier` 加入 InAppOnly 分支。`inbox_link` 回 `/lists/{payload.wishlist_id}/edit`。`render` 四語，zh-Hant 文案固定為「{supporter_name} 把你的清單「{list_title}」放進了他們的願望」。

`notify_targets` 用 CTE 先 `UPDATE ... SET support_notified_at = now() WHERE wishlist_id = $1 AND funding_mode = 'support' AND support_notified_at IS NULL AND deleted_at IS NULL AND support_wishlist_id IS NOT NULL RETURNING support_wishlist_id`，再 JOIN 對方清單與收件人 INSERT。收件人：個人清單是 `owner_id`；組織清單是 `org_members` 的有效 owner、coordinator，欄位名依 `org_members` 實際定義。

`item_create` 在清單 `status = 'active'` 時呼叫；`PATCH` 發佈分支在 draft → active 成功後呼叫，都在同一交易。

- [ ] **Step 4: 更新 `docs/notification-policy.md`**

把 `wishlist.supported` 加進分類與第四級表格。

- [ ] **Step 5: 執行測試確認通過**

Run: `cargo test --manifest-path apps/api/Cargo.toml --test support_items && cargo test --manifest-path apps/api/Cargo.toml --lib notify`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add apps/api/src apps/api/tests/support_items.rs docs/notification-policy.md
git commit -m "feat(api): 被設為支持對象時發送站內通知"
```

### Task 5: 創建者介面

**Files:**
- Modify: `apps/web/app/components/creator/ItemSheet.vue`、`apps/web/app/pages/lists/[id]/edit.vue:184`、`apps/web/app/pages/lists/[id]/progress.vue:104`
- Create: `apps/web/app/utils/supportTarget.ts`
- Modify: `apps/web/i18n/locales/{zh-Hant,en,vi,id}/creator.json`、`lists.json`
- Test: `apps/web/tests/supportTarget.test.ts`

**Interfaces:**
- Consumes: Task 2 的 API，Task 3 的 `support` 物件。
- Produces: `export const parseListSlug = (input: string): string | null`。

- [ ] **Step 1: 寫失敗測試**

```ts
expect(parseListSlug('https://wishly.tw/s/abc123?x=1')).toBe('abc123')
expect(parseListSlug('  abc123 ')).toBe('abc123')
expect(parseListSlug('https://evil.test/x y')).toBeNull()
expect(parseListSlug('')).toBeNull()
```

- [ ] **Step 2: 執行確認失敗**

Run: `cd apps/web && npx vitest run tests/supportTarget.test.ts`
Expected: FAIL

- [ ] **Step 3: 實作 `parseListSlug` 與 `ItemSheet`**

取得方式新增第三個選項「支持一份清單」，只在新增時顯示。選定後顯示連結輸入框，失焦時以 `GET /public/wishlists/{slug}` 帶出對方標題，成功且標題欄為空時預填；失敗顯示錯誤並停用儲存。品牌、規格、網址、數量、單價欄位隱藏。

`funding_mode` 初始化要保留 `'support'`。`save()` 的 body：support 新增時送 `funding_mode`、`support_slug`、`title`、`priority`，加上救災清單的 `category`、`urgency`；編輯時不送 `qty_needed`、`funding_mode`。編輯時目標唯讀顯示。

- [ ] **Step 4: 修改 `edit.vue` 與 `progress.vue`**

`edit.vue` 第 184 行加 support 分支，顯示對方名稱或「已結束」，並顯示 `support.points`、`support.qty`，null 時顯示 `–`。`progress.vue` 第 104 行加 support 分支，顯示相同數字，不顯示進度條和捐助者。

- [ ] **Step 5: 驗證**

Run: `cd apps/web && npx vitest run tests/supportTarget.test.ts tests/i18nMessages.test.ts && npm run typecheck`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add apps/web
git commit -m "feat(web): 在品項面板建立支持品項"
```

### Task 6: 分享頁與歸因

**Files:**
- Create: `apps/web/app/utils/supportVia.ts`、`apps/web/app/components/guest/SupportCard.vue`
- Modify: `apps/web/app/pages/s/[slug].vue:73,246`、`apps/web/app/components/guest/ClaimSheet.vue:34`、`apps/web/app/components/guest/FundSheet.vue:32`
- Modify: `apps/web/i18n/locales/{zh-Hant,en,vi,id}/guest.json`、`apps/web/app/assets/guest.css`
- Test: `apps/web/tests/supportVia.test.ts`、`apps/web/tests/guestCards.test.ts`

**Interfaces:**
- Produces:
  - `export function rememberVia(slug: string, raw: unknown, now = Date.now()): void`
  - `export function currentVia(slug: string, now = Date.now()): string | undefined`

- [ ] **Step 1: 寫失敗測試**

`supportVia.test.ts`：

- 存入後 `currentVia` 回傳該 id。
- 第二次 `rememberVia` 覆蓋第一次。
- 7 天整還有效，7 天加 1 毫秒回 `undefined`。
- 不同 slug 互不影響。
- 非 UUID 不寫入。
- 內容損毀、`localStorage` 拋例外時回 `undefined`，不拋錯。

`guestCards.test.ts`：

- 未結束時顯示數字和連到 `/s/{slug}?via={id}` 的連結。
- `ended` 時無連結，顯示「已結束」和數字。
- `points`、`qty` 為 null 時不顯示數字列。
- `closed` 時無連結。

- [ ] **Step 2: 執行確認失敗**

Run: `cd apps/web && npx vitest run tests/supportVia.test.ts tests/guestCards.test.ts`
Expected: FAIL

- [ ] **Step 3: 實作 `supportVia.ts`**

鍵為 `ws_via:{slug}`，值為 `JSON {id, at}`，全部包 try/catch。

- [ ] **Step 4: 實作 `SupportCard.vue`**

Props：`item: any, closed: boolean, index?: number`。依選定的方案 A：等寬小標籤「支持清單」、標題、對方名稱與封面、雙數字列、紅色主要按鈕「前往支持」。沿用 `g-card`、`g-item`、`g-btn`。無進度條，無百分比。

- [ ] **Step 5: 接上分享頁**

`[slug].vue`：`v-for` 改用 `<template>`，support 品項渲染 `GuestSupportCard`；`onMounted` 呼叫 `rememberVia(slug, route.query.via)`；`viaId` 在 `onMounted` 後以 `currentVia(slug)` 取得。把 `:via="viaId"` 傳給 `ClaimSheet`、`FundSheet`，兩者的 POST body 加 `via_item_id: props.via`。`isDone` 把支持品項視為未完成，維持原順序。

- [ ] **Step 6: 驗證**

Run: `cd apps/web && npm run typecheck && npm run test:coverage && npm run build`
Expected: PASS，覆蓋率不低於 baseline。

- [ ] **Step 7: Commit**

```bash
git add apps/web
git commit -m "feat(web): 分享頁顯示支持品項並記錄歸因"
```

### Task 7: 文件、mock 與端對端驗收

**Files:**
- Modify: `docs/04-data-schema-api-contract.html`、`CLAUDE.md`、`apps/web/mocks/server.mjs`
- Create: `apps/web/e2e/flow-f25-support-item.spec.ts`

**Interfaces:**
- Consumes: Task 1–6 全部。

- [ ] **Step 1: 寫 e2e**

網紅建立公開清單並放入支持品項，指向另一位使用者的公開清單；粉絲開網紅分享頁，點「前往支持」，登入後贊助或認捐；回到網紅分享頁，10 秒快取在測試環境不存在，直接斷言數字出現，頁面不含粉絲暱稱。對方把清單改私人後，品項顯示「已結束」，頁面不含對方標題。

- [ ] **Step 2: 執行 e2e**

Run: `cd apps/web && COMPOSE_PROJECT_NAME=wishlist npx playwright test e2e/flow-f25-support-item.spec.ts`
Expected: PASS

- [ ] **Step 3: 更新文件與 mock**

契約文件補 `support_slug`、`via_item_id`、`support` 物件和新錯誤碼。`CLAUDE.md` 架構段落加「支持品項」一項，寫明 `via_item_id` 無外鍵、彙總在 `support.rs`、含支持品項的快取規則。`mocks/server.mjs` 的 demo 清單加一個支持品項。

- [ ] **Step 4: 全量驗證**

Run: `cargo fmt --manifest-path apps/api/Cargo.toml --check && cargo clippy --manifest-path apps/api/Cargo.toml --all-targets -- -D warnings && cargo test --manifest-path apps/api/Cargo.toml`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add docs CLAUDE.md apps/web/mocks apps/web/e2e
git commit -m "docs: 更新支持品項契約、架構說明與端對端測試"
```
