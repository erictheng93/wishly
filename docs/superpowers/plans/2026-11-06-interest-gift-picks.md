# 依興趣推薦禮物 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 精選商品標興趣；`GET /users/{handle}/gift-picks` 依對方看得到的興趣推薦商品（配不到給一般推薦）；朋友個人頁顯示推薦區塊、情境頁可選「幫哪位朋友挑」並置頂符合商品。

**Architecture:** 後端在 `catalog_products` 加 `interests`，新模組 `gift_picks.rs` 讀觀看者身分（no-store），可見性沿用 `profiles::Rel` / `can_see`（Rel 計算抽成共用函式）。前端抽出興趣 chip 元件供設定頁與後台共用；`CatalogGrid` 加選填 `matched` prop；個人頁與情境頁在 client 端呼叫推薦 API。

**Tech Stack:** Rust axum + sqlx（PostgreSQL 16）、Nuxt 4 + vue-i18n、vitest、`sqlx::test`、Playwright。

**Spec:** `docs/superpowers/specs/2026-11-06-interest-gift-picks-design.md`

## Global Constraints

- 每個任務在自己的 worktree（off `main` 或指定分支）；e2e 用 `COMPOSE_PROJECT_NAME=wishlist`。
- commit 不加 Claude / Co-Authored-By 署名。
- 後端提交前：`cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、相關 `--test`，看真實結束碼。前端：`npm run typecheck`、`npx vitest run`、`npm run test:coverage`、`npm run build`。
- migration：`0072_catalog_interests.sql`。
- `/public/catalog/products` 仍不可讀 cookie（CDN 快取）；推薦 API 一律 `Cache-Control: no-store`。
- 推薦結果不得受觀看者看不到的興趣 / 不想收到 / 生日影響（spec 反目標）。
- 新錯誤碼要補 `apps/web/i18n/locales/*/errors.json`（`tests/i18nMessages.test.ts` 檢查）；新文案四語（zh-Hant / en / vi / id）。
- UI：紅白實驗室 token、`UiIcon`、無 eyebrow / 編號 / Unicode 符號圖示。

## Review Focus

1. 興趣設為「摯友」、觀看者只是好友：推薦必須是 fallback，`matched` 不可出現該興趣 — Task 2 `close_interests_hidden_from_friend`。
2. 「不想收到」設為只有自己：不可因此排除商品 — Task 2 `hidden_dislikes_do_not_filter`。
3. 匿名觀看者看 public 興趣的人：可得 interests 推薦；看 friends 興趣的人：fallback — Task 2 `anonymous_sees_public_only`。
4. 情境頁「載入更多」後置頂商品不可重複出現 — Task 4 `mergePicks dedupes across pages`。
5. 目錄全部沒有 interests（上線初期）：個人頁仍有 fallback 卡片，不是空區塊 — Task 2 `fallback_when_catalog_untagged`。

---

### Task 1: 商品興趣欄位（後端）

**Files:**
- Create: `db/migrations/0072_catalog_interests.sql`
- Modify: `apps/api/src/catalog.rs`（`PCOLS`、product struct 加 `interests: Vec<String>`、`merge` 用 `vocab(m, "interests", crate::interests::INTERESTS)`、INSERT / UPDATE 綁 interests、`admin()` 與 `public_products` JSON 回 `interests`）
- Test: `apps/api/tests/catalog.rs`

**Interfaces:**
- Produces: `catalog_products.interests text[] NOT NULL DEFAULT '{}'`；公開與後台商品 JSON 都有 `"interests": [..]`。

- [ ] **Step 1: Failing tests**：`admin_sets_interests`（POST 帶 `["coffee","tea"]` → 201 且回應含之；PATCH 改 `["travel"]` → 200）；`admin_rejects_unknown_interest`（`["nope"]` → 422，pointer `/interests`）；`public_products_include_interests`。
- [ ] **Step 2:** `cargo test ... --test catalog` → FAIL。
- [ ] **Step 3:** 實作。
- [ ] **Step 4:** `--test catalog --test catalog_upload` → PASS；fmt、clippy。
- [ ] **Step 5:** Commit `feat(api): 精選商品可標興趣（0072）`。

---

### Task 2: 推薦 API（後端）

**Files:**
- Create: `apps/api/src/gift_picks.rs`、`apps/api/tests/gift_picks.rs`
- Modify: `apps/api/src/profiles.rs`（把 101–135 行的封鎖判斷與 `Rel` 計算抽成 `pub(crate) async fn relation_of(pool: &PgPool, owner: Uuid, viewer: Option<Uuid>) -> Result<Relation, AppError>`，`profile()` 改用它）、`apps/api/src/lib.rs`（merge routes）

**Interfaces:**
- Consumes: Task 1 的 `catalog_products.interests`；`profiles::Rel`、`can_see`；`friends::days_until`；`interests::INTERESTS`。
- Produces: `pub(crate) struct Relation { pub rel: Rel, pub blocked_by_owner: bool, pub i_blocked: bool, pub friend: bool }`（`friend` 供 profiles 既有 relation 字串沿用）。
- Produces route: `GET /users/{handle}/gift-picks?occasion=&limit=` → `{ "basis": "interests"|"fallback", "products": [ {ref,title,spec,note,price_amount,image_url,product_url,recipients,occasions,interests,brand,matched:[..]} ] }`，`Cache-Control: no-store`。`occasion` 須在 `OCCASIONS`（422 `/occasion`）；`limit` 1–50（422 `/limit`），預設 6。
- 規則照 spec「推薦 API」1–6；生日可見性沿用 profiles 既有規則（`birthday_visibility` public / friends / 本人）。`blocked_by_owner` → 404；`i_blocked` → `{"basis":"fallback","products":[]}`。
- 商品 JSON 形狀與 `public_products` 相同：把 catalog.rs 的列→JSON 轉換抽成 `pub(crate) fn product_json(..) -> Value` 共用，不複製。

- [ ] **Step 1: Failing tests**（`tests/gift_picks.rs`，用 `common::user/befriend`，商品直接 SQL INSERT）：
  - `ranks_by_interest_overlap`：對方 interests `["coffee","tea"]`（public）；商品 A `["coffee","tea"]`、B `["coffee"]`、C `[]` → basis interests，順序 A、B，C 不在；A.matched == ["coffee","tea"]。
  - `close_interests_hidden_from_friend`：interests_vis=close、觀看者是好友非摯友 → basis fallback，所有 matched 為空。
  - `anonymous_sees_public_only`：interests_vis=public → 匿名得 interests；改 friends → 匿名得 fallback。
  - `dislikes_exclude_by_title_and_brand`：gift_dislikes `["香氛蠟燭"]`（gift_prefs_vis=public）→ 標題含「香氛蠟燭」的商品不出現；品牌名含「香氛蠟燭」亦不出現。
  - `hidden_dislikes_do_not_filter`：gift_prefs_vis=private → 該商品照常出現。
  - `single_char_dislike_ignored`：dislike `["茶"]` 不排除「茶具組」。
  - `fallback_when_catalog_untagged`：商品都無 interests → basis fallback，回 recipients 含 friend 的商品依 sort_order。
  - `fallback_prefers_birthday_within_30_days`：觀看者看得到的生日在 10 天內、未帶 occasion → fallback 只含 occasions 含 birthday 的商品。
  - `occasion_filters_candidates`：`?occasion=housewarming` 只回該場合商品。
  - `blocked_cases`：對方封鎖觀看者 → 404；觀看者封鎖對方 → `{basis:"fallback",products:[]}`。
  - `no_store_and_validation`：回應 `cache-control: no-store`；`occasion=x` 422、`limit=51` 422。
  - `profile_still_works`：既有 `--test profiles --test profile_fields` 全綠（重構後回歸）。
- [ ] **Step 2:** `--test gift_picks` → FAIL。
- [ ] **Step 3:** 實作（users 一次查詢取 interests / interests_vis / gift_dislikes / gift_prefs_vis / birthday / birthday_visibility；商品一次查詢取全部 active（目錄小），排序在應用層）。
- [ ] **Step 4:** `--test gift_picks --test profiles --test profile_fields --test catalog` → PASS；fmt、clippy。
- [ ] **Step 5:** Commit `feat(api): 依興趣推薦精選商品（gift-picks）`。

---

### Task 3: 共用興趣選擇元件、後台標籤、商品格標記（前端）

**Files:**
- Create: `apps/web/app/components/InterestPicker.vue`（props `modelValue: string[]`、`max?: number`；依 `INTEREST_GROUPS` 分組 chip，沿用 GiftPrefsCard 64–69 行的 markup 與 `data-chip`、`aria-pressed`、滿額停用）
- Modify: `apps/web/app/components/creator/profile/GiftPrefsCard.vue`（改用 `InterestPicker`，行為不變）、`apps/web/app/pages/admin/catalog.vue`（商品表單加「適合的興趣」= `InterestPicker` 不設 max；送出帶 `interests`）、`apps/web/app/utils/catalog.ts`（`CatalogProduct` 加 `interests?: string[]`；新型別 `GiftPick = CatalogProduct & { matched: string[] }`）、`apps/web/app/components/guest/CatalogGrid.vue`（選填 prop `matched?: Record<string, string[]>`，有值且該 ref 非空時卡片顯示等寬小標籤列「符合：咖啡、茶」，用 `$t('interests.tag.<id>')`）、i18n `admin.json`（`admin.catalog.interests`）與 `guest.json`（`guest.inspire.matched`）四語
- Test: `apps/web/tests/interestPicker.test.ts`、擴充 `apps/web/tests/profileCards.test.ts`（GiftPrefs 行為不變）

**Interfaces:**
- Produces: `<InterestPicker v-model="ids" :max="10" />`；`CatalogGrid` 的 `matched` prop；`GiftPick` 型別。

- [ ] **Step 1: Failing tests**：`interestPicker.test.ts` — 點 chip 觸發 `update:modelValue` 加入 / 移除；`max=2` 選滿後其餘 `disabled`；不設 max 時全可選。`CatalogGrid` 給 `matched={ r1: ['coffee'] }` 時 r1 卡片顯示「咖啡」標記、r2 無標記；不傳 `matched` 時兩張都無標記。
- [ ] **Step 2:** vitest → FAIL。
- [ ] **Step 3:** 實作。
- [ ] **Step 4:** typecheck、vitest、test:coverage、build → 0。
- [ ] **Step 5:** Commit `feat(web): 共用興趣選擇元件、後台商品標興趣、商品格符合標記`。

---

### Task 4: 個人頁推薦區塊與情境頁「幫哪位朋友挑」（前端）

**Files:**
- Create: `apps/web/app/utils/giftPicks.ts`（`fetchGiftPicks(apiBase, handleOrId, opts: { occasion?: string; limit?: number }): Promise<{ basis: 'interests'|'fallback'; products: GiftPick[] }>`，`credentials: 'include'`、`cache: 'no-store'`、`headers: nativeHeaders()`；`mergePicks(picks: GiftPick[], rest: CatalogProduct[]): CatalogProduct[]` 以 ref 去重、picks 在前）
- Modify: `apps/web/app/components/creator/ProfileSections.vue` 或 `pages/u/[handle].vue`（興趣區塊後加推薦區塊：本人、blocked、API 失敗時不顯示；`basis` 決定標題「適合送給 {name} 的禮物」/「也許 {name} 會喜歡」；用 `GuestCatalogGrid` 並傳 `matched`；若 `/u` 未載入 `guest.css` 則在該頁 import）、`apps/web/app/pages/inspire/[scene].vue`（只在 `scene.occasion` 存在且已登入時顯示「幫哪位朋友挑？」`<select>`，選項來自 `GET /friends`，預設「不指定」；選擇同步到 `?for=<user_id>`（`router.replace`），讀取 `route.query.for` 還原；選定後 `fetchGiftPicks(id, { occasion: scene.occasion, limit: 50 })`，`basis: interests` → 列表用 `mergePicks`，傳 `matched`；`fallback` → 不重排，顯示一行提示；SSR 輸出不變）、i18n `me.json`、`guest.json` 四語
- Test: `apps/web/tests/giftPicks.test.ts`、e2e `apps/web/e2e/gift-picks.spec.ts`

**Interfaces:**
- Consumes: Task 2 API；Task 3 `CatalogGrid.matched`、`GiftPick`。
- 與 spec 的差異（計畫決定）：沒有 `occasion` 的情境頁本來就不顯示商品，所以選單只在有 `occasion` 的情境出現。

- [ ] **Step 1: Failing tests**：
  - `giftPicks.test.ts` — `mergePicks dedupes across pages`：picks [r2, r5]、rest [r1, r2, r3, r5, r6] → [r2, r5, r1, r3, r6]；`fetchGiftPicks` 帶 `occasion` / `limit` query 且 `credentials: 'include'`。
  - e2e `gift-picks.spec.ts`（真 API）：staff 以 `/admin/catalog` API 給兩個商品標 `coffee`；A 設 handle、interests `["coffee"]` public；B 登入看 `/u/{A.handle}` → 推薦區塊出現且卡片有「咖啡」標記；B 開 `/inspire/birthday?for=<A.id>` → 第一張卡是標了 coffee 的商品、有標記；A 把 interests_vis 改 private → B 重新整理個人頁看到 fallback 標題。
- [ ] **Step 2:** vitest → FAIL。
- [ ] **Step 3:** 實作。
- [ ] **Step 4:** typecheck、vitest、test:coverage、build；e2e `gift-picks.spec.ts` + 既有 `rich-profile.spec.ts`、任何 inspire 相關 spec（grep `inspire`）→ PASS。
- [ ] **Step 5:** Commit `feat(web): 個人頁送禮推薦、情境頁幫朋友挑`。

---

### Task 5: 收尾

- [ ] **Step 1:** `CLAUDE.md` 架構段補一條：`catalog_products.interests`、`GET /users/{handle}/gift-picks`（no-store、只用 `can_see` 看得到的興趣 / 不想收到 / 生日、fallback 規則）、`profiles::relation_of` 共用、`CatalogGrid.matched`、情境頁 `?for=`。
- [ ] **Step 2:** 全量後端測試、fmt、clippy；前端 typecheck、test:coverage、build；看真實結束碼。
- [ ] **Step 3:** Commit `docs: CLAUDE.md 補依興趣推薦禮物`。開 PR。
