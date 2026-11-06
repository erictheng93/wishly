# 會員個人資料擴充 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 帳號摯友名單（取代每份清單的 selected 名單）、送禮偏好、興趣標籤、重要日子（含 7 天前站內通知）、居住 / 預設收件地址、年齡、外部連結與 `/l/{handle}` 連結頁、個人頁封面。

**Architecture:** 後端每塊各一個 migration 與模組（`close_friends.rs`、`user_dates.rs`、`user_links.rs`、`user_addresses.rs`、`interests.rs`），純量欄位併入既有 `PATCH /me`（`auth.rs::patch_me`）；可見性在 `profiles.rs` 依觀看者過濾。前端設定頁拆成 `components/creator/profile/*Card.vue`，個人頁 `/u/[handle]` 加區塊，新 SSR 頁 `/l/[handle]`。

**Tech Stack:** Rust axum + sqlx（PostgreSQL 16）、Nuxt 4 + vue-i18n、vitest、`sqlx::test`。

**Spec:** `docs/superpowers/specs/2026-11-06-rich-profile-design.md`（執行者必讀；本計畫只寫決定，不重述 spec 的理由）

## Global Constraints

- 在獨立 worktree 作業（`superpowers:using-git-worktrees`），不動主資料夾；e2e 需 `COMPOSE_PROJECT_NAME=wishlist`。
- commit 不加 Claude 署名（使用者偏好覆蓋預設 attribution）。
- 每個任務提交前：`cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、相關 `cargo test --test <slice>` 看**真實結束碼**（不可接 `| tail` 判斷）；前端任務跑 `npm run typecheck && npx vitest run`。
- migration 從 `0067` 起依任務順序：0067 close_friends、0068 profile_fields、0069 user_addresses、0070 user_dates、0071 user_links。
- 新 enum：`CREATE TYPE profile_vis AS ENUM ('public','friends','close','private')`（0068 建立；0067 不需要）。
- 可見性判斷一律走 Task 3 的 `profiles::can_see(vis, rel)`；看不到的欄位**不出現在 JSON**（不是 null）。
- 稽核 diff 不放地址、電話、Email、IP；地址只記 `{"changed": true}`。
- 新使用者內容欄位要進帳號刪除（`account.rs` 442 行附近的 UPDATE 與刪表）與 `/me/export`。
- UI：紅白實驗室 token、`UiIcon`、不加 eyebrow / 編號 / Unicode 符號圖示、新文案四語（zh-Hant / en / vi / id）。
- 外部連結 `<a target="_blank" rel="nofollow ugc noopener noreferrer">`；後端只收 `https`、≤500 字、可解析、無 userinfo，伺服器**不抓**網址。

## Review Focus

1. 被封鎖 / 封鎖對方時，個人頁不可回任何新欄位（偏好、標籤、日子、地址、年齡、連結、封面以外）——Task 3 測 `blocked_profile_hides_rich_fields`。
2. 摯友不是好友了（解除好友後）仍殘留可見性——Task 1 測 `unfriend_removes_close_both_ways`，Task 3 測 `close_field_hidden_after_unfriend`。
3. `/public/links/{handle}` 不可帶 cookie 也不可因 cookie 回更多（CDN 快取）——Task 6 測 `public_links_ignores_cookie`。
4. 生日 2/29、日子 02-29 在非閏年的倒數與通知——Task 5 測 `feb29_date_notifies_in_non_leap_year`（沿用 `friends::days_until` 的夾日行為）。
5. 舊前端 / 舊快取仍呼叫 `/wishlists/{id}/allowed-users`——Task 1 測 `allowed_users_api_removed` 回 404（不是 500）。

---

### Task 1: 帳號摯友名單（後端）

**Files:**
- Create: `db/migrations/0067_close_friends.sql`, `apps/api/src/close_friends.rs`, `apps/api/tests/close_friends.rs`
- Modify: `apps/api/src/lib.rs`（merge routes）、`access.rs:72-80`、`birthday.rs:49`、`blocks.rs:63-67`、`friends.rs:391-393, 512-518, 579`、`inspire.rs:79-80`、`notify.rs:719`、`profiles.rs:158, 197`、`wishlists.rs:33-34, 2177-` （刪 `allowed_get` / `allowed_put` / `allowed_list`）
- Modify tests: `tests/visibility.rs`、`tests/friend_invite_wishlist.rs`、`tests/birthday_notify.rs`、`tests/blocks.rs`（凡 INSERT `wishlist_allowed_users` 或打 allowed-users 的改用 `close_friends`）
- Modify: `scripts/mutate.py`（109–110、195–196 兩個 invite mutant 的 search 字串改成新 SQL；新增 2 個 mutant 見 Step 7）

**Interfaces:**
- Produces: 表 `close_friends(owner_id uuid, friend_id uuid, created_at timestamptz, PK(owner_id, friend_id))`，FK users ON DELETE CASCADE，`CREATE INDEX close_friends_friend_idx ON close_friends(friend_id)`。
- 各查詢直接寫 `EXISTS (SELECT 1 FROM close_friends c WHERE c.owner_id = <owner> AND c.friend_id = <viewer>)`，不另做 SQL 常數。
- Produces: `pub async fn is_close(pool: &PgPool, owner: Uuid, viewer: Uuid) -> Result<bool, AppError>`（Task 3 用）。
- Produces routes: `GET /me/close-friends` → `{"users":[{id,display_name,handle,avatar_url}]}`（依 created_at）；`PUT /me/close-friends/{user_id}` → 204；`DELETE /me/close-friends/{user_id}` → 204。上限 `const MAX_CLOSE: i64 = 200`，超過 422 `code=LIMIT`；非好友 422 `code=NOT_FRIEND`；對自己 422。
- Produces audit actions：`close_friends.add` / `close_friends.remove`（target 對方 user id，用 `security::audit` 同型 helper）。

- [ ] **Step 1: Migration 0067**
  - 順序：①`UPDATE wishlists SET visibility='private' WHERE visibility='selected' AND org_id IS NULL AND NOT EXISTS (allowed 名單中仍為好友者)`；②`UPDATE wishlists SET visibility='private' WHERE visibility='selected' AND org_id IS NOT NULL`（spec：預期 0 列）；③CREATE TABLE；④`INSERT INTO close_friends SELECT DISTINCT w.owner_id, a.user_id ... WHERE w.org_id IS NULL AND 仍為好友 ON CONFLICT DO NOTHING`；⑤`DROP TABLE wishlist_allowed_users`。
  - 「仍為好友」= `EXISTS (SELECT 1 FROM friendships f WHERE f.user_a = LEAST(w.owner_id, a.user_id) AND f.user_b = GREATEST(...))`。

- [ ] **Step 2: Write failing tests in `tests/close_friends.rs`**
  - `put_requires_friendship`：非好友 PUT → 422 `NOT_FRIEND`；好友 PUT → 204，GET 列出 1 人；重複 PUT 仍 204。
  - `selected_list_visible_to_close_only`：A 建 selected 清單；好友 B 非摯友 → `/restricted/wishlists/{slug}` 403；A PUT B 為摯友 → 200；另一位 A 的好友 C 只是 D 的摯友（不是 A 的）→ 仍 403。
  - `unfriend_removes_close_both_ways`：A、B 互為摯友後 B 解除好友 → 兩方向 `close_friends` 皆 0 列。
  - `block_removes_close_both_ways`：同上改封鎖。
  - `invite_accept_on_selected_adds_close`：selected 清單的好友邀請被 C 接受 → `close_friends(A, C)` 存在。
  - `allowed_users_api_removed`：`GET /wishlists/{id}/allowed-users` → 404。
  - `limit_200`：直接 SQL 塞 200 列後再 PUT → 422 `LIMIT`。
  - `migration_0067_merges_and_privatizes`：`#[sqlx::test(migrations = false)]`，照 `tests/birthday_notify.rs:313` `migration_0054_does_not_resend_after_deploy` 的手法（排序 migration 檔、先套 `< "0067"`、塞資料、再套其餘）。斷言：有勾且仍是好友 → `close_friends` 有列、清單仍 selected；名單為空 → 清單變 private；勾的人已非好友 → 不搬、清單變 private；同一擁有者兩份清單勾不同人 → 兩人都進名單。

- [ ] **Step 3: Run** `cargo test --manifest-path apps/api/Cargo.toml --test close_friends` → FAIL（表 / 路由不存在）。

- [ ] **Step 4: 實作 `close_friends.rs` 與改寫所有讀寫點**
  - PUT / DELETE 在交易內先 `friends::lock_pair(&mut tx, u.id, other)`（鎖序規則：此鎖最先取），PUT 再檢查 friendships、計數、INSERT ON CONFLICT DO NOTHING。
  - `friends.rs:391` 與 `blocks.rs:64` 改為 `DELETE FROM close_friends WHERE (owner_id=$1 AND friend_id=$2) OR (owner_id=$2 AND friend_id=$1)`。
  - `friends.rs:515` 改為 `INSERT INTO close_friends (owner_id, friend_id) VALUES ($1, $2) ON CONFLICT DO NOTHING`，bind 清單擁有者與 `u.id`。
  - 其餘讀取點：`a.wishlist_id = w.id AND a.user_id = X` → `c.owner_id = w.owner_id AND c.friend_id = X`。`birthday.rs:49` 的 `$2` 是 wishlist id，改用 `$1`（owner）。

- [ ] **Step 5: 更新受影響的既有測試**，跑 `--test visibility --test friend_invite_wishlist --test birthday_notify --test blocks --test social --test inspire --test profiles --test close_friends` → 全 PASS。

- [ ] **Step 6: 更新 `scripts/mutate.py`** 109–110 的 search 字串為新 INSERT；新增：
  - `close-friends-owner-scope`：`access.rs` 中 `c.owner_id = ` 條件改為 `TRUE`，killed by `close_friends::selected_list_visible_to_close_only`。
  - `close-friends-unfriend-cleanup`：`friends.rs` 的 DELETE close_friends 改為空字串，killed by `unfriend_removes_close_both_ways`。
  - 跑 `scripts/mutate.sh close-friends-owner-scope close-friends-unfriend-cleanup invite-list-allowed-on-conflict invite-list-selected-only` → 全 killed、還原後 SHA 一致。

- [ ] **Step 7: fmt + clippy（看結束碼）後 commit** `feat(api): 帳號摯友名單取代清單 selected 名單（0067）`

---

### Task 2: 摯友名單（前端）

**Files:**
- Modify: `apps/web/app/components/creator/VisibilityPicker.vue`（移除 `users` model 與好友勾選；selected 時顯示「N 位摯友」＋ `NuxtLink to="/settings#close-friends"`）
- Modify: `apps/web/app/pages/lists/new.vue:8,26,59`、`pages/lists/[id]/edit.vue:62,114`（移除 allowed 相關）
- Create: `apps/web/app/components/creator/profile/CloseFriendsCard.vue`
- Modify: `pages/settings.vue`（掛 `<section id="close-friends">`）、`pages/friends.vue`（每列「設為摯友 / 取消摯友」切換）
- Modify i18n: `apps/web/i18n/locales/{zh-Hant,en,vi,id}/creator.json`（`vis.selected.desc` 改為「只有你的摯友名單能開，對方需要登入。」、`pickTitle` 改 `closeCount`）、`settings.json`（`closeFriends.*`）
- Test: `apps/web/tests/closeFriends.test.ts`

**Interfaces:**
- Consumes: Task 1 的三支 `/me/close-friends` API。
- Produces: composable `useCloseFriends()` in `composables/useCloseFriends.ts` → `{ ids: Ref<Set<string>>, list: Ref<any[]>, load(): Promise<void>, toggle(id: string, on: boolean): Promise<void> }`（`friends.vue` 與 `CloseFriendsCard` 共用）。

- [ ] **Step 1: Failing test** `closeFriends.test.ts`：mock `useApi`，`toggle('u1', true)` 呼叫 `PUT /me/close-friends/u1` 且 `ids` 含 `u1`；`toggle('u1', false)` 呼叫 DELETE 且移除；API 失敗時 `ids` 還原。
- [ ] **Step 2: Run** `npx vitest run tests/closeFriends.test.ts` → FAIL。
- [ ] **Step 3: 實作** composable、卡片（好友清單中勾選；空好友時同現有 `noFriendsPre/Post` 文案）、picker、friends 列切換、移除兩頁的 allowed 呼叫。
- [ ] **Step 4: Run** `npm run typecheck && npx vitest run` → PASS；`npm run test:coverage` 不低於 baseline。
- [ ] **Step 5: Commit** `feat(web): 摯友名單管理，清單摯友改用帳號名單`

---

### Task 3: 純量個人欄位 + 個人頁可見性（後端）

**Files:**
- Create: `db/migrations/0068_profile_fields.sql`, `apps/api/src/interests.rs`, `apps/api/tests/profile_fields.rs`
- 與 spec 的差異（計畫決定）：個人封面**不**新增 upload purpose，沿用 `cover`（同存 `covers/`），因為掛圖是在 PATCH 時寫入、presign 不知道目標；只需讓 `retire_media` 認得 `users.cover_image_key`。
- Modify: `auth.rs:659-700`（GET /me 回新欄位）、`auth.rs:713-`（`PatchMe` 與 `patch_me`）、`profiles.rs`（新欄位與過濾）、`uploads.rs:536-541`（`retire_media` 加 `users.cover_image_key`）、`account.rs:364`（刪帳號時納入 cover key）、`account.rs:442`（清空新欄位）、`account.rs:115`（export）

**Interfaces:**
- 0068 欄位（`users`）：`gift_likes text[] NOT NULL DEFAULT '{}'`、`gift_dislikes text[] NOT NULL DEFAULT '{}'`、`gift_prefs_vis profile_vis NOT NULL DEFAULT 'friends'`、`interests text[] NOT NULL DEFAULT '{}'`、`interests_vis profile_vis NOT NULL DEFAULT 'public'`、`birth_year smallint CHECK (birth_year BETWEEN 1900 AND 2100)`、`age_vis profile_vis NOT NULL DEFAULT 'private' CHECK (age_vis <> 'public')`、`home_address_vis profile_vis NOT NULL DEFAULT 'private' CHECK (home_address_vis <> 'public')`、`cover_preset text CHECK (同 0032 的值)`、`cover_image_key text`、`CHECK (cover_preset IS NULL OR cover_image_key IS NULL)`。
- Produces in `interests.rs`: `pub const INTERESTS: &[&str]`（40 個 kebab id，與前端 `utils/interests.ts` 相同順序）；`pub fn valid(ids: &[String]) -> bool`。
- Produces in `profiles.rs`: `pub(crate) enum Rel { Anonymous, Other, Friend, Close, SelfView }` 與 `pub(crate) fn can_see(vis: &str, rel: Rel) -> bool`（public→全部；friends→Friend|Close|SelfView；close→Close|SelfView；private→SelfView）。Task 4、5 使用。
- PATCH /me 新鍵：`gift_likes`、`gift_dislikes`（string[]，各 ≤20 條、trim 後 1–40 字、去重保序，否則 422 `/gift_likes/{i}`）、`gift_prefs_vis`、`interests`（≤10 且全在 INTERESTS，否則 422 `ENUM`）、`interests_vis`、`birth_year`（null 清除；> 今年 422）、`age_vis`（只收 friends/close/private）、`home_address_vis`（同）、`cover_preset`（null 清除）、`cover_image_key`（`covers/` 前綴、`uploads::valid_key`；設定時清 preset，反之亦然；換掉舊 key 時在同交易 `uploads::request_retire`，新 key 用 `media_tx` + `remember_media`，比照 `wishlists.rs:724-726`）。
- GET /me 回傳全部新欄位原值，加 `cover_image_url`。
- `GET /users/{handle}` 的 `user` 物件新增（只在 `can_see` 時出現）：`gift_likes`、`gift_dislikes`、`interests`、`age`（整數）；永遠出現：`cover_preset`、`cover_image_url`。封鎖分支（`profiles.rs:85` 附近）不加任何新欄位。

- [ ] **Step 1: Failing tests in `tests/profile_fields.rs`**
  - `patch_validates_gift_lists`：21 條 → 422；41 字 → 422；`[" a ","a"]` → 存成 `["a"]`。
  - `interests_must_be_in_library`：`["not-a-tag"]` → 422；11 個 → 422；合法 3 個 → 200。
  - `age_vis_rejects_public`：`age_vis:"public"` → 422。
  - `visibility_matrix`：A 設 `gift_prefs_vis=close`、`interests_vis=friends`、`birth_year=1990, birthday=05-01, age_vis=friends`；匿名看不到三者；非好友看不到；好友看得到 interests 與 age 但無 gift_likes 鍵；摯友三者皆有；本人三者皆有。用 `assert!(body["user"].get("gift_likes").is_none())` 斷言「不出現」。
  - `age_computed_from_birthday`：固定今天的話用 `birth_year = 今年-30` 且 birthday 為明天 → `age == 29`。（年齡計算抽成 `fn age(today: NaiveDate, year: i16, mmdd: Option<&str>) -> i32` 並在模組內單測）。
  - `blocked_profile_hides_rich_fields`：B 封鎖 A 後 A 看 B → 404；A 封鎖 B 後 A 看 B → `user` 只有既有 5 個鍵。
  - `close_field_hidden_after_unfriend`：摯友解除好友後看不到 close 欄位。
  - `cover_key_not_retired_while_referenced`：PATCH cover_image_key 後呼叫 retire 流程（同 `tests/account_media_cleanup.rs` 手法）→ 不刪。
  - `account_delete_clears_profile_fields`、`export_includes_profile_fields`。
- [ ] **Step 2: Run** `cargo test ... --test profile_fields` → FAIL。
- [ ] **Step 3: 實作** migration、`interests.rs`、`can_see` / `Rel`、PATCH/GET /me、profiles.rs、uploads/account 修改。`Rel` 在 profiles.rs 由既有 `relation` 與 `close_friends::is_close(uid, viewer)` 推導（只在 friend 時查一次）。
- [ ] **Step 4: Run** `--test profile_fields --test profiles --test account_deletion --test account_media_cleanup --test exports_notifications` → PASS。
- [ ] **Step 5: fmt + clippy 後 commit** `feat(api): 送禮偏好、興趣、年齡、封面欄位與個人頁可見性（0068）`

---

### Task 4: 居住地址與預設收件地址（後端）

**Files:**
- Create: `db/migrations/0069_user_addresses.sql`, `apps/api/src/user_addresses.rs`, `apps/api/tests/user_addresses.rs`
- Modify: `lib.rs`、`profiles.rs`（home 地址）、`account.rs`（刪除、匯出）、`wishlists.rs`（新路由 `POST /wishlists/{id}/shipping-address/from-default`，放在既有 shipping-address handler 旁）

**Interfaces:**
- 表 `user_addresses(user_id uuid FK users ON DELETE CASCADE, kind text CHECK (kind IN ('shipping','home')), recipient_name_enc bytea, phone_enc bytea, address_enc bytea NOT NULL, updated_at timestamptz NOT NULL DEFAULT now(), PRIMARY KEY (user_id, kind), CHECK (kind='home' OR (recipient_name_enc IS NOT NULL AND phone_enc IS NOT NULL)))`。
- 驗證與遮罩沿用既有清單 shipping-address handler 的函式（找出 `PUT /wishlists/{id}/shipping-address` 的驗證 / 遮罩邏輯，抽成 `pub(crate)` 共用，不複製）。
- Routes：`GET /me/addresses` → `{"shipping": {recipient_name, phone, address}|null, "home": {address}|null}`（遮罩值）；`PUT /me/addresses/shipping` body `{recipient_name, phone, address}`；`PUT /me/addresses/home` body `{address}`；`DELETE /me/addresses/{kind}` → 204。
- `POST /wishlists/{id}/shipping-address/from-default`：需 `can_manage_wishlist`；帳號無 shipping → 409 `NO_DEFAULT_ADDRESS`；成功時以 SQL 直接複製三個 `_enc` 欄位（upsert 到 `shipping_addresses`），回應同 `GET /wishlists/{id}/shipping-address`（遮罩）。
- profiles.rs：`can_see(home_address_vis, rel)` 時 `user.home_address = 明文`（`sealed::open_str`，解不開就省略該鍵並 `tracing::warn!`）。
- 稽核：`profile.address_update` / `profile.address_delete`，diff `{"kind": "...", "changed": true}`。

- [ ] **Step 1: Failing tests**：`put_get_masks`；`home_visible_to_close_only`（home_address_vis=close：好友無鍵、摯友有明文）；`shipping_never_on_profile`（任何身分都沒有 shipping 相關鍵）；`from_default_copies_without_plaintext`（回應為遮罩、`shipping_addresses` 解密後等於原值）；`from_default_requires_manage`（別人的清單 → 404）；`from_default_without_address_409`；`audit_has_no_address`（`audit_logs.diff::text` 不含地址字串）；`account_delete_removes_addresses`。
- [ ] **Step 2: Run** `--test user_addresses` → FAIL。
- [ ] **Step 3: 實作。**
- [ ] **Step 4: Run** `--test user_addresses --test crowdfund_owner --test account_deletion` → PASS。
- [ ] **Step 5: fmt + clippy 後 commit** `feat(api): 居住地址與預設收件地址（0069）`

---

### Task 5: 重要日子與 7 天前站內通知（後端）

**Files:**
- Create: `db/migrations/0070_user_dates.sql`, `apps/api/src/user_dates.rs`, `apps/api/tests/user_dates.rs`
- Modify: `lib.rs`、`birthday.rs`（`birthday_tick_at` 結尾呼叫 `crate::user_dates::date_tick_at(pool, today)` 並累加）、`notify.rs`（`KINDS`、`category`、`email_tier`、寄前重驗、`render` 四語、`inbox_link`）、`docs/notification-policy.md`、`profiles.rs`、`account.rs`（匯出）

**Interfaces:**
- 表 `user_dates(id uuid PK DEFAULT gen_random_uuid(), user_id uuid NOT NULL FK users ON DELETE CASCADE, title text NOT NULL CHECK (char_length(title) BETWEEN 1 AND 30), month_day text NOT NULL CHECK (同 0014 birthday regex), year smallint CHECK (year BETWEEN 1900 AND 2100), recurring boolean NOT NULL DEFAULT true, visibility profile_vis NOT NULL DEFAULT 'friends', created_at timestamptz NOT NULL DEFAULT now(), CHECK (recurring OR year IS NOT NULL))`；`CREATE INDEX ON user_dates(user_id)`。
- 表 `user_date_sent(date_id uuid FK user_dates ON DELETE CASCADE, year smallint, PRIMARY KEY (date_id, year))`。
- Routes：`GET /me/dates` → `{"dates":[{id,title,month_day,year,recurring,visibility,past:boolean}]}`；`POST /me/dates`（≤20 筆，超過 422 `LIMIT`；MM-DD 合法性用 auth.rs 生日同一個檢查函式）；`PATCH /me/dates/{id}`（改 month_day / year / recurring / visibility 時同交易 `DELETE FROM user_date_sent WHERE date_id=$1 AND year >= 今年`）；`DELETE /me/dates/{id}`。非本人的 id → 404。
- `pub async fn date_tick_at(pool: &PgPool, today: NaiveDate) -> Result<u64, sqlx::Error>`：候選 = visibility <> 'private'、擁有者未刪除 / 停權、`days_until(today, month_day) <= 7`、非 recurring 時 `year == year_of(today, d)`；每筆一個交易，`INSERT INTO user_date_sent ... ON CONFLICT DO NOTHING RETURNING` 成功才 `INSERT INTO notifications (user_id, channel, kind, payload) SELECT r.id, 'email', 'profile.date_soon', jsonb_build_object('date_id', $id, 'owner_id', $owner)`，收件人 = 好友（visibility public/friends）或摯友（close），排除刪除 / 停權 / 任一方向封鎖 / `inbox_social = 'false'`。`year_of` 從 birthday.rs 改為 `pub(crate)`。
- notify：`category` → Social（加進 `matches!(head, "friend" | "birthday")` 那行，用 `kind == "profile.date_soon"`）；`email_tier` → InAppOnly；寄前重驗：日子仍存在、visibility 不是 private、收件人仍可見（同 tick 條件），否則 `done!("cancelled", ...)`；`inbox_link` → `/u/{handle}`，擁有者無 handle 時 `/u/{owner_id}`；`render` 標題：zh「{who} 的「{title}」快到了」、en「{who}'s "{title}" is coming up」、vi「Sắp đến "{title}" của {who}」、id「"{title}" milik {who} sudah dekat」，內文含 M/D。
- profiles.rs：`user.dates = [{title, month_day, year?, days_until}]`，只列 `can_see` 且未過的（非 recurring 已過不列），依 days_until 排序；`year` 只在 recurring=false 時回。

- [ ] **Step 1: Failing tests**：`crud_limit_and_ownership`；`notifies_friends_7_days_before`（固定 today；friends 可見 → 好友收到 1 則、非好友 0、本人 0）；`close_date_notifies_only_close`；`private_date_never_notifies`；`idempotent_same_year`（tick 兩次只 1 則）；`late_tick_still_sends_within_window`（today = 日子前 3 天、未發 → 發）；`one_off_past_not_sent`；`edit_date_resets_sent`；`blocked_recipient_skipped`；`inbox_social_off_skipped`；`feb29_date_notifies_in_non_leap_year`；`worker_cancels_if_made_private`（enqueue 後改 private，跑 worker → status cancelled）。notify 既有單元測試會檢查 KINDS 齊全與 link 以 `/` 開頭——一併跑。
- [ ] **Step 2: Run** `--test user_dates` → FAIL。
- [ ] **Step 3: 實作**，含 `docs/notification-policy.md` 新增一行（Social、InAppOnly、7 天前、對象規則）。
- [ ] **Step 4: Run** `--test user_dates --test birthday_notify --test notifications --test notify_governance` 與 `cargo test --lib notify` → PASS。
- [ ] **Step 5: fmt + clippy 後 commit** `feat(api): 重要日子與 7 天前站內通知（0070）`

---

### Task 6: 外部連結與公開連結端點（後端）

**Files:**
- Create: `db/migrations/0071_user_links.sql`, `apps/api/src/user_links.rs`, `apps/api/tests/user_links.rs`
- Modify: `lib.rs`、`profiles.rs`、`account.rs`（匯出）

**Interfaces:**
- 表 `user_links(id uuid PK DEFAULT gen_random_uuid(), user_id uuid NOT NULL FK users ON DELETE CASCADE, title text NOT NULL CHECK (char_length(title) BETWEEN 1 AND 40), url text NOT NULL CHECK (char_length(url) <= 500 AND url LIKE 'https://%'), position smallint NOT NULL, hidden boolean NOT NULL DEFAULT false, created_at timestamptz NOT NULL DEFAULT now())`；`CREATE INDEX ON user_links(user_id, position)`。
- `pub(crate) fn valid_url(s: &str) -> bool`：`url::Url::parse` 成功、scheme `https`、有 host、`username()` 空且 `password()` None、長度 ≤ 500（模組內單測：`http://`、`https://a:b@x.com`、`javascript:`、`https://`、500+ 字皆 false）。
- Routes：`GET /me/links` → `{"links":[{id,title,url,hidden}]}`（依 position）；`PUT /me/links` body `{"links":[{title,url,hidden}]}`（≤10，整組覆寫：同交易 DELETE + INSERT，position = 陣列索引；錯誤指標 `/links/{i}/url`）。
- `GET /public/links/{handle}`：**不呼叫 `access::viewer`、不讀 cookie**；`handle = lower($1)`、未刪除未停權，否則 404；回 `{"user":{display_name, handle, bio, avatar_url, cover_preset, cover_image_url}, "links":[{title,url}]}`（hidden 排除）；標頭 `Cache-Control: public, s-maxage=60`，不帶 ETag。
- profiles.rs：`user.links = [{title,url}]`（非 hidden，任何未封鎖觀看者都有）。

- [ ] **Step 1: Failing tests**：`put_replaces_and_orders`；`rejects_bad_urls`（http、userinfo、11 個 → 422）；`public_links_hides_hidden`；`public_links_ignores_cookie`（帶 A 本人 cookie 也看不到 hidden，且回應無 `set-cookie`、`cache-control` 等於 `public, s-maxage=60`）；`public_links_404_for_suspended`；`profile_includes_links`。
- [ ] **Step 2: Run** `--test user_links` → FAIL。
- [ ] **Step 3: 實作。**
- [ ] **Step 4: Run** `--test user_links --test profiles` → PASS。
- [ ] **Step 5: fmt + clippy 後 commit** `feat(api): 外部連結與公開連結頁端點（0071）`

---

### Task 7: 設定頁新卡片（前端）

**Files:**
- Create: `apps/web/app/components/creator/profile/{AppearanceCard,GiftPrefsCard,DatesCard,PrivateInfoCard}.vue`、`apps/web/app/utils/interests.ts`、`apps/web/app/utils/linkIcon.ts`、`apps/web/i18n/locales/{zh-Hant,en,vi,id}/interests.json`
- Modify: `apps/web/app/pages/settings.vue`（`#social` 卡移除 bio；依序掛 AppearanceCard → GiftPrefsCard → DatesCard → PrivateInfoCard → CloseFriendsCard，置於 `#social` 之後）、`components/creator/CoverPicker.vue`（不改 API：purpose 維持 `cover`，個人封面一樣存 `covers/`）、`settings.json` 四語、`components/ui/UiIcon` 缺的圖示（instagram、threads、youtube、line、facebook、tiktok、x、github、link，1.5px 線條）
- Test: `apps/web/tests/interests.test.ts`、`apps/web/tests/linkIcon.test.ts`、`apps/web/tests/profileCards.test.ts`

**Interfaces:**
- Consumes: Task 3 PATCH/GET /me 鍵、Task 4 `/me/addresses*`、Task 5 `/me/dates*`、Task 6 `/me/links`。
- Produces `utils/interests.ts`：`export const INTEREST_GROUPS: { id: string; tags: string[] }[]`（6 組，id：`life`,`food`,`sports`,`arts`,`tech`,`family`；tags 合計 40，與後端 `INTERESTS` 同序同值）、`export const MAX_INTERESTS = 10`。
- Produces `utils/linkIcon.ts`：`export function linkIcon(url: string): string`（host 比對，含子網域，如 `www.instagram.com`、`youtu.be`；無效 URL 回 `link`）。
- 各卡片各自 `api('/me', { method: 'PATCH', body })` 或各自 API，成功後 `user.value = 回應`（同現有 `saveSocial`）；錯誤用 `apiErrMsg`，欄位錯誤依 `errors[].pointer` 顯示。
- PrivateInfoCard 選單文案：「在個人頁顯示：不顯示（只有我）/ 好友 / 摯友」，值 `private / friends / close`；地址輸入後清空明文、只顯示遮罩（照 `ShippingForm.vue` 作法）。

- [ ] **Step 1: Failing tests**
  - `interests.test.ts`：tags 共 40 且不重複；每個 tag 在四語 `interests.json` 都有翻譯；（後端常數同步：讀 `apps/api/src/interests.rs` 文字，正則抽出 id 陣列，與前端扁平陣列相等）。
  - `linkIcon.test.ts`：`https://www.instagram.com/x`→`instagram`、`https://youtu.be/x`→`youtube`、`https://line.me/R/ti/p/x`→`line`、`https://example.com`→`link`、`not a url`→`link`。
  - `profileCards.test.ts`：GiftPrefsCard 新增第 21 條時新增鈕停用；互動標籤選滿 10 個時其餘 chip `disabled`；DatesCard recurring 取消勾選時年份欄變必填。
- [ ] **Step 2: Run** `npx vitest run tests/interests.test.ts tests/linkIcon.test.ts tests/profileCards.test.ts` → FAIL。
- [ ] **Step 3: 實作四張卡片**（排序用上下移動按鈕，不加拖曳套件）、AppearanceCard 內含 CoverPicker、bio、連結列表、「複製連結頁網址」（`${siteOrigin()}/l/${handle}`，無 handle 時提示先設 handle）。
- [ ] **Step 4: Run** `npm run typecheck && npm run test:coverage && npm run build` → 全過。
- [ ] **Step 5: Commit** `feat(web): 設定頁個人頁外觀、送禮偏好、重要日子、地址與年齡卡片`

---

### Task 8: 個人頁區塊、連結頁、收件地址帶入（前端）

**Files:**
- Modify: `apps/web/app/pages/u/[handle].vue`（區塊順序依 spec「個人頁結構」）
- Create: `apps/web/app/pages/l/[handle].vue`
- Modify: `apps/web/nuxt.config.ts`（`/l/**` 為 SSR、不加入創建者 `ssr:false` 清單、不加入 `LOCALIZABLE` 與 sitemap）
- Modify: `apps/web/app/components/creator/ShippingForm.vue`（尚無收件資訊且 `GET /me/addresses` 有 shipping 時顯示「使用預設收件地址」按鈕 → `POST .../from-default`，成功 `emit('saved', r)`）
- Modify: `me.json` 四語（個人頁區塊標題）、`creator.json`（`shipping.useDefault`）
- Test: `apps/web/tests/profileSections.test.ts`、e2e `apps/web/e2e/rich-profile.spec.ts`

**Interfaces:**
- Consumes: Task 3–6 的 `GET /users/{handle}` 新鍵（缺鍵 = 不顯示區塊）與 `GET /public/links/{handle}`。
- `/l/[handle]`：`useFetch(${apiBase}/api/v1/public/links/${handle}, { credentials: 'omit', server: true })`；SEO：title = 名稱、ogDescription = bio、ogImage = avatar 或 og-default、`robots: 'noindex'`；版面：封面 → 頭像 → 名稱 → bio → 連結按鈕（全寬、`UiIcon :name="linkIcon(url)"`）→ 固定按鈕「看我的願望清單」→ `/u/{handle}`；404 顯示同 `/u` 的 gone 樣式。
- 個人頁新增「分享連結頁」按鈕（`navigator.share` / 複製 `${siteUrl}/l/${handle}`，沿用 `share()` 寫法）。

- [ ] **Step 1: Failing tests**
  - `profileSections.test.ts`：給一個不含 `gift_likes`、`home_address` 的 profile → 不渲染對應區塊標題；含 `dates` → 顯示最近一筆與 `days_until`；`links` 的 `<a>` 有 `rel="nofollow ugc noopener noreferrer"` 與 `target="_blank"`。
  - e2e `rich-profile.spec.ts`：註冊 A 設 handle、加一個連結與一筆偏好（friends）→ 匿名開 `/l/{handle}` 看到連結、開 `/u/{handle}` 看不到偏好；B 成為好友後看得到偏好。
- [ ] **Step 2: Run** `npx vitest run tests/profileSections.test.ts` → FAIL。
- [ ] **Step 3: 實作。**
- [ ] **Step 4: Run** `npm run typecheck && npm run test:coverage && npm run build`；e2e：`COMPOSE_PROJECT_NAME=wishlist npx playwright test e2e/rich-profile.spec.ts` → PASS。
- [ ] **Step 5: Commit** `feat(web): 個人頁新區塊、/l 連結頁、預設收件地址帶入`

---

### Task 9: 收尾

- [ ] **Step 1:** 更新 `CLAUDE.md` 架構段：新增一條「個人資料擴充（0067–0071）」摘要——摯友 = `close_friends`（取代 `wishlist_allowed_users`）、`profile_vis` 與 `profiles::can_see`、`/public/links/{handle}` 不可讀 cookie、`profile.date_soon` 為 InAppOnly、地址永不進稽核 diff。並把「清單可見性」一段的「selected＝被選中的好友/摯友」改為「selected＝帳號摯友名單」。
- [ ] **Step 2:** 全量 `cargo test --manifest-path apps/api/Cargo.toml`、`cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、前端 `npm run typecheck && npm run test:coverage && npm run build`，看真實結束碼。
- [ ] **Step 3:** `scripts/mutate.sh all` 背景跑（不必等完成才開 PR；開 PR 前確認原始碼已還原）。
- [ ] **Step 4:** Commit `docs: CLAUDE.md 補個人資料擴充`，開 PR（描述列出標籤庫 40 個 id 給使用者確認）。
