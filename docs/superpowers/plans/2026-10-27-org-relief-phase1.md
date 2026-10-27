# 公益組織（org）救災：第一段 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 讓公益組織能申請、被平台審核、管理成員、建立或加入災害事件、發布公開的據點清單，並接受物資認捐與點數眾籌；志工可登記實收。

**Architecture:** 新增後端模組 `orgs.rs`（申請、文件、許可、成員、邀請、據點、實收）與 `relief_events.rs`（事件）。審核放在 `admin.rs` 旁的新檔 `org_admin.rs`。據點就是 `type='relief'` 的 wishlist，**沿用既有的清單、品項、認捐、眾籌端點**。授權由 SQL 函式 `can_manage_wishlist(owner_id, org_id, uid)` 統一判斷：org 清單看成員角色，個人清單看 owner。眾籌出貨沿用 `points::capture_and_order`，做法是建立據點時同時寫入 `shipping_addresses`。前端新增 `/org`、`/org/apply`、`/orgs/[id]/**`、`/e/**`、`/o/[slug]` 和兩個後台頁，據點品項沿用 `/lists/[id]` 編輯器。

**Tech Stack:** Rust（axum + sqlx，`#[sqlx::test]`）、PostgreSQL 16、Nuxt 4 + vitest + Playwright。

**Spec:** `docs/superpowers/specs/2026-10-27-org-relief-registration-design.md`（與既有 `docs/01`、`02`、`04` 的 P2-D 規格一起讀）

## 與 spec 的偏離（第一段刻意縮小，交付時要告知產品負責人）

1. **不用 PostGIS／互動地圖**：事件的地區存縣市陣列＋文字補充，據點只存地址並附 Google Maps 連結。`/public/relief/sites?bbox` 地圖 API 延後。這樣也就不需要 `CREATE EXTENSION`。
2. **組織文件不另開私有 bucket**：API 收檔後用 `sealed::seal` 加密，再存進既有 bucket 的 `org-docs/` 路徑。就算公開網址被猜到，拿到的也只是密文；下載時由 API 解密。
3. **對還沒有帳號的人，邀請不寄信**：API 回傳邀請連結，由管理者自己轉傳（和好友邀請連結同一模式）。對方已經是使用者時，另外透過 notify outbox 寄通知信。

## Global Constraints

- 鎖序沿用 CLAUDE.md：`wishlists`（FOR SHARE）→ `point_wallets` → `wishlist_items` → `claims` / `contributions`。新的鎖序：`organizations` → `org_members`；`relief_events` 依 id 由小到大。不可顛倒。
- 寫入型的認捐和實收需要 `Idempotency-Key`，沿用 `idempotency.rs`，占位和業務寫入在同一交易。
- 錯誤一律用 `AppError`（problem+json），新錯誤碼見各 task。
- 新增或修改 `FOR UPDATE` / `FOR SHARE` / `ON CONFLICT` 時，要在 `scripts/mutate.py` 的 `CASES` 補 mutant，並確認 `scripts/mutate.sh all` 全數 killed。
- 新 migration 檔名從 `db/migrations/0041_` 開始。
- 創建者路由要加進 `apps/web/nuxt.config.ts` 的 `routeRules` `ssr:false` 清單。直接 `$fetch` 的受保護請求要帶 `nativeHeaders()`。
- 公開端點（`/public/**`）不可讀 cookie，可被 CDN 快取。清單不再公開時呼叫 `cdn::purge_wishlists`。
- i18n 四個語系（`zh-Hant`、`en`、`id`、`vi`）都要補 key，`id` 和 `vi` 可以先放英文。
- 樣式用 `assets/tokens.css`（紅白實驗室）。組織模式的標記用紅色 #c8102e。
- commit 訊息不加 Co-Authored-By 等 Claude 署名（使用者偏好）。
- 在自己的 worktree 裡開發（使用者偏好）。worktree 跑 e2e 要設定 `COMPOSE_PROJECT_NAME=wishlist`。
- 數值：審核目標 3 個工作天（只是文案）、退件後 30 天才能重送、文件每個 ≤10MB（`application/pdf`、`image/jpeg`、`image/png`）、邀請 7 天有效、事件查重窗口 ±7 天且同一縣市、relief 認捐 `claim_ttl_hours` 預設 48。

## Review Focus

1. **停用或移除的工作人員**拿舊連結編輯據點或品項 → 404（不是 403，不洩漏資源存在）。測試在 Task 6。
2. **兩位 owner 同時互相降級**，最後一個人必須被擋下，組織不能沒有 owner。測試在 Task 5（併發）。
3. **據點綁定的勸募許可過期** → 背景工作把據點轉為 `paused`，新認捐和眾籌回 409 `SITE_PAUSED`，已在進行的不受影響。測試在 Task 9。
4. **冒名組織用相同統編送審**，而正牌組織還在審核中 → 409 `TAX_ID_TAKEN`。測試在 Task 2。
5. **捐助者打開已被合併的事件網址** → 回應帶 `merged_into`，前端轉到新事件，原本的據點連結照常運作。測試在 Task 8、Task 16。

---

### Task 1: Migration — 組織、事件、據點欄位

**Files:**
- Create: `db/migrations/0041_orgs_relief.sql`
- Test: `apps/api/tests/org_schema.rs`

**Interfaces:**
- Produces（表／函式，後續 task 都依此）：
  - `org_role` enum `('owner','coordinator','volunteer')`
  - `organizations(id uuid pk default uuid_v7(), slug char(10) unique, legal_name text 1..100, display_name text 1..50, org_type text CHECK IN ('association','foundation','religious','other'), tax_id char(8) CHECK ~ '^[0-9]{8}$', registration_no text, authority text, website text, intro text ≤2000, logo_key text, rep_name text, contact_name text, contact_title text, contact_phone_enc bytea, public_email text, public_phone text, verification_status text NOT NULL DEFAULT 'draft' CHECK IN ('draft','pending','needs_changes','verified','rejected'), review_note text, flagged_fields text[] NOT NULL DEFAULT '{}', resubmit_after timestamptz, submitted_at timestamptz, verified_at timestamptz, created_by uuid NOT NULL REFERENCES users ON DELETE RESTRICT, created_at, updated_at)`
    - `CHECK ((verification_status='verified') = (verified_at IS NOT NULL))`
    - 送審必填欄位：draft 以外的狀態，`legal_name`、`display_name`、`org_type`、`tax_id`、`registration_no`、`authority`、`rep_name`、`contact_name`、`contact_phone_enc`、`public_email` 都要 NOT NULL（一條 CHECK）。草稿階段可以有空欄位，所以這些欄位本身允許 NULL，只有 `legal_name` 和 `display_name` 在 draft 也必填。
    - `UNIQUE INDEX organizations_tax_id_live ON organizations(tax_id) WHERE verification_status IN ('pending','needs_changes','verified')`
    - `UNIQUE INDEX organizations_one_draft ON organizations(created_by) WHERE verification_status='draft'`
  - `org_members(org_id fk CASCADE, user_id fk users CASCADE, role org_role, handle text ≤30, disabled_at timestamptz, created_at, PRIMARY KEY(org_id,user_id))`，加上 `INDEX (user_id)`
  - `org_invites(id, org_id fk CASCADE, email text CHECK = lower(email), role org_role, handle text, token_hash bytea UNIQUE, invited_by uuid fk users, expires_at, accepted_at, cancelled_at, created_at)`
  - `org_documents(id, org_id fk CASCADE, kind text CHECK IN ('registration','permit'), object_key text, content_type text, size_bytes int, uploaded_by uuid, created_at)`
  - `org_permits(id, org_id fk CASCADE, permit_no text 1..60, valid_from date, valid_to date CHECK (valid_to >= valid_from), document_id fk org_documents, status text DEFAULT 'pending' CHECK IN ('pending','approved','rejected'), review_note text, created_at)`
  - `relief_events(id, slug char(10) unique, name text 1..80, hazard_type text CHECK IN ('earthquake','typhoon','flood','fire','other'), counties text[] NOT NULL CHECK (cardinality(counties) >= 1), area_note text ≤200, occurred_on date, description text ≤2000, created_by_org fk organizations RESTRICT, merged_into fk relief_events, moderation_status moderation_status DEFAULT 'ok', moderation_reason text, created_at, updated_at)`
  - `wishlists` 新增：`org_id uuid fk organizations RESTRICT`、`event_id uuid fk relief_events`、`permit_id uuid fk org_permits`、`site_address text`、`site_status text CHECK IN ('active','paused','closed')`
    - `CHECK wishlists_relief_site`：`type='relief'` ⇔ 上面五欄全部 NOT NULL；非 relief 時全部為 NULL
    - `CHECK wishlists_relief_public`：`type <> 'relief' OR visibility = 'public'`
  - `wishlist_items` 新增：`category text CHECK IN ('water','food','medical','hygiene','shelter','clothing','baby','other')`、`urgency text CHECK IN ('critical','high','medium','low')`
  - trigger `items_relief_shape`：relief 清單的品項，`category` 和 `urgency` 都必填；非 relief 的品項這兩欄都必須是 NULL（`check_violation`）。**兩種 funding_mode 都允許**。
  - trigger `wishlists_relief_publish`（BEFORE INSERT OR UPDATE OF status, org_id, permit_id）：relief 清單的 `status='active'` 必須同時滿足 org 為 `verified`，以及 permit 為 `approved` 且 `current_date` 在有效期間內。否則 `RAISE EXCEPTION USING ERRCODE='check_violation'`（這是安全網，應用層會先檢查）。
  - `receipts(id, item_id fk wishlist_items CASCADE, wishlist_id fk wishlists CASCADE, qty int CHECK > 0, note text ≤200, recorded_by uuid fk users, created_at)`
  - SQL 函式 `can_manage_wishlist(p_owner uuid, p_org uuid, p_uid uuid) RETURNS boolean STABLE`：`p_org IS NULL` 時回傳 `p_owner = p_uid`；否則回傳 `EXISTS org_members(org_id=p_org, user_id=p_uid, role IN ('owner','coordinator'), disabled_at IS NULL)`
  - `set_updated_at` trigger 掛在 `organizations` 和 `relief_events`

- [ ] **Step 1: 寫 `apps/api/tests/org_schema.rs` 失敗測試**（`#[sqlx::test(migrations = "../../db/migrations")]`，直接用 SQL 操作）

```rust
// relief_site_requires_org_event_permit_address_status：INSERT type='relief' 少 org_id → Err（check_violation）
// relief_must_be_public：relief + visibility='friends' → Err
// relief_item_requires_category_urgency：relief 清單加品項缺 urgency → Err；registry 品項帶 category → Err；relief crowdfund 品項（帶 category/urgency）→ Ok
// relief_cannot_go_active_unless_verified_and_permit_valid：org 為 pending 時 UPDATE status='active' → Err；org verified + permit approved 但 valid_to=昨天 → Err；兩者都有效 → Ok
// tax_id_unique_among_live_orgs：兩個 pending 用同一個 tax_id → 第二筆 Err；一筆 rejected 加一筆 pending → Ok
// one_draft_per_creator：同一 created_by 建兩個 draft → 第二筆 Err
// can_manage_wishlist_matrix：assert 個人清單 owner=true／他人=false；org 清單的 owner 角色=true、coordinator=true、volunteer=false、disabled coordinator=false、非成員但為 owner_id 的建立者=false
```

- [ ] **Step 2:** `cargo test --manifest-path apps/api/Cargo.toml --test org_schema`，預期 FAIL（表不存在）
- [ ] **Step 3:** 寫 migration。註解用繁中，引用 spec 章節。在 `apps/api/tests/common/mod.rs` 加 fixture：`pub async fn org(pool, owner: Uuid, status: &str) -> Uuid`（verified 時一併設定 `verified_at`）、`pub async fn permit(pool, org: Uuid, status: &str, valid_to_offset_days: i32) -> Uuid`、`pub async fn member(pool, org, user, role: &str)`、`pub async fn event(pool, org) -> (Uuid, String)`、`pub async fn site(pool, org, creator, event, permit, status: &str) -> TestList`
- [ ] **Step 4:** 跑 `--test org_schema` 和 `--test migration_constraints`，全部 PASS
- [ ] **Step 5:** 在 `scripts/mutate.py` 的 SQL CHECK 清單加入 `("check-relief-site", "0041_orgs_relief.sql", "wishlists_relief_site", "relief_site_requires_org_event_permit_address_status")`，然後 commit：`feat(db): 組織、救災事件、據點欄位與 can_manage_wishlist`

### Task 2: 組織申請（草稿、送審、撤回、/me）

**Files:**
- Create: `apps/api/src/orgs.rs`
- Modify: `apps/api/src/lib.rs`（`pub mod orgs;`、`.merge(orgs::routes())`）、`apps/api/src/auth.rs:640-646`（`/me` 的 `orgs`）
- Test: `apps/api/tests/org_apply.rs`

**Interfaces:**
- Produces：
  - `pub async fn member_role(ex, org: Uuid, user: Uuid) -> Result<Option<String>, AppError>`：只看未停用的成員
  - `pub async fn require_role(ex, org: Uuid, user: Uuid, min: &str) -> Result<String, AppError>`：`min` 是 `"volunteer" < "coordinator" < "owner"`；非成員或已停用回 `AppError::NotFound`，角色不足回 403 `ORG_ROLE_REQUIRED`
  - `pub async fn audit_user(ex, user: Uuid, action: &str, entity: &str, id: Option<Uuid>, diff: Value) -> Result<(), AppError>`：寫入 `audit_logs`，`actor_type='user'`；diff 裡帶 `org_id`
  - 路由：
    - `POST /orgs`：body 為申請欄位，`contact_phone` 明文 → `sealed::seal`；建立 draft 並在同一交易加入 owner 成員；201，回傳 org JSON
    - `GET /orgs/{id}`：成員才能看；JSON 含 `role`、`verification_status`、`review_note`、`flagged_fields`、`resubmit_after`；`contact_phone` 只回給 owner
    - `PATCH /orgs/{id}`：owner；僅限 draft 或 needs_changes
    - `POST /orgs/{id}/submit`：owner；檢查必填、至少一份 registration 文件、至少一張 permit
    - `POST /orgs/{id}/withdraw`：owner；pending → draft
  - org JSON 欄位名稱和表欄位相同（除了 `contact_phone`），前端依此實作

- [ ] **Step 1: 寫失敗測試**

```rust
// create_draft_makes_caller_owner：POST /orgs → 201；GET /me 的 orgs[0] == {id, name: display_name, role:"owner", verification_status:"draft"}
// second_draft_conflicts：同一人再 POST /orgs → 409 ORG_DRAFT_EXISTS
// submit_requires_fields_documents_permit：缺 tax_id → 422 VALIDATION_FAILED pointer "/tax_id"；欄位齊全但沒有文件 → 422 pointer "/documents"；沒有 permit → 422 pointer "/permits"；全部齊全 → 200 status "pending"、submitted_at 不為 null
// tax_id_taken_by_live_org：A org pending 用 12345675；B 用同一統編 submit → 409 TAX_ID_TAKEN
// invalid_tax_id_checksum：統編檢查碼錯 → 422 pointer "/tax_id" code "TAX_ID_CHECKSUM"
// needs_changes_only_flagged_fields_editable：用 SQL 把狀態設為 needs_changes、flagged_fields={intro}；PATCH intro → 200；PATCH legal_name → 422 code "FIELD_LOCKED"
// resubmit_blocked_until_resubmit_after：rejected 且 resubmit_after=now()+1 day；PATCH → 409 RESUBMIT_TOO_EARLY；把 resubmit_after 設為過去 → PATCH 會把狀態回到 draft 並 200
// withdraw_returns_to_draft
// non_member_gets_404：其他使用者 GET /orgs/{id} → 404
// volunteer_cannot_patch：403 ORG_ROLE_REQUIRED
```

- [ ] **Step 2:** `cargo test ... --test org_apply`，FAIL
- [ ] **Step 3:** 實作。統編檢查碼放在 `validate.rs`，新增 `pub fn tax_id_ok(s: &str) -> bool`：採財政部 2023 新制，權重 1,2,1,2,1,2,4,1，總和能被 5 整除；第 7 碼為 7 時另有特例（總和 +1 後能被 5 整除也算通過），並附單元測試 `#[test] tax_id_checksum`，案例 `"04595257"` 為 true。`409 TAX_ID_TAKEN` 由 unique violation（`organizations_tax_id_live`）轉換。rejected → draft 的重新編輯在 PATCH 裡處理：先檢查 `resubmit_after`。
- [ ] **Step 4:** 跑 `--test org_apply`，PASS
- [ ] **Step 5:** Commit：`feat(api): 組織申請草稿、送審與 /me orgs`

### Task 3: 組織文件與勸募許可

**Files:**
- Modify: `apps/api/src/orgs.rs`
- Test: `apps/api/tests/org_documents.rs`

**Interfaces:**
- Consumes：`require_role`、`sealed::seal/open`、`uploads::S3`（presign PUT／GET 走 `s3_req`；若它是私有函式，就在 `uploads.rs` 加 `pub async fn put_object(key, bytes, ct)` 和 `pub async fn get_object(key) -> Vec<u8>`）
- Produces：
  - `POST /orgs/{id}/documents?kind=registration|permit`：owner；request body 是原始檔案，`Content-Type` 必須是 pdf／jpeg／png，≤10MB（這條路由用 `DefaultBodyLimit::max(10*1024*1024 + 1)`）；先 `seal` 再存到 `org-docs/{org_id}/{uuid}`；201 `{id, kind, content_type, size_bytes}`
  - `GET /orgs/{id}/documents/{doc_id}`：owner／coordinator 或 staff；解密後回傳原檔，`Cache-Control: no-store`、`Content-Disposition: attachment`
  - `DELETE /orgs/{id}/documents/{doc_id}`：owner，只能在 draft 或 needs_changes
  - `POST /orgs/{id}/permits {permit_no, valid_from, valid_to, document_id}`：owner；201，status 為 pending
  - `GET /orgs/{id}/permits`：成員可看
- 已驗證的組織新增 permit 時，permit 保持 pending，由 staff 個別審核（Task 4）

- [ ] **Step 1: 寫失敗測試**

```rust
// upload_seals_bytes_at_rest：上傳 PDF bytes → 從 MinIO 直接讀 object_key 的內容 != 原始 bytes；GET 下載 == 原始 bytes
// rejects_wrong_type_and_oversize：text/html → 422 pointer "/content_type"；10MB+1 → 413 或 422 "/content_length"
// download_denied_to_volunteer_and_strangers：volunteer → 403；陌生人 → 404；staff → 200
// permit_requires_document_of_same_org：document_id 屬於別的 org → 422 pointer "/document_id"
// permit_dates_validated：valid_to < valid_from → 422
```

- [ ] **Step 2–4:** 依序跑 FAIL → 實作 → PASS（`--test org_documents`；這組測試需要 MinIO 在跑，參照 `tests/uploads.rs` 的做法）
- [ ] **Step 5:** Commit：`feat(api): 組織文件加密保存與勸募許可登錄`

### Task 4: 平台審核（組織、許可、撤銷）

**Files:**
- Create: `apps/api/src/org_admin.rs`（在 `lib.rs` merge）
- Modify: `apps/api/src/notify.rs`（新增 kind `org.review` 的 `render_lang`，四語系）
- Test: `apps/api/tests/org_review.rs`

**Interfaces:**
- Consumes：`admin::{Staff, audit, PageQ, limit, page}`、`cdn::purge_wishlists`
- Produces：
  - `GET /admin/orgs?status=`：分頁，沿用 `PageQ`
  - `GET /admin/orgs/{id}`：含解密後的 `contact_phone`、文件列表、permits、成員
  - `POST /admin/orgs/{id}/review {decision: "approve"|"needs_changes"|"reject", note, flagged_fields?}`
    - 交易內先 `SELECT ... FROM organizations WHERE id=$1 FOR UPDATE`
    - approve：限 pending；至少要有 1 張 approved permit，否則 409 `PERMIT_REQUIRED`；設定 `verified_at`
    - needs_changes：限 pending；`flagged_fields` 不可為空，且每個值都要是可編輯欄位名或 `"documents"` / `"permits"`
    - reject：可從 pending 或 verified 退件；設定 `resubmit_after = now() + 30 days`。若原本是 verified（撤銷），同一交易內把 `type='relief' AND org_id=$1 AND status='active'` 的據點改為 `draft`，commit 後對這些 slug 呼叫 `cdn::purge_wishlists`
    - 每個決定都寫 `admin::audit` 和 `org.review` 通知（寄給所有未停用的 owner）
  - `POST /admin/permits/{id}/review {decision: "approve"|"reject", note}`
  - `GET /admin/orgs` 預設排序：`submitted_at ASC`（先送的先審）

- [ ] **Step 1: 寫失敗測試**

```rust
// approve_requires_approved_permit：pending org 只有 pending permit → 409 PERMIT_REQUIRED；先 approve permit 再 approve org → 200，verification_status "verified"
// needs_changes_sets_flagged_fields：flagged_fields ["intro","documents"] → org 的 flagged_fields 等於這兩個；空陣列 → 422
// reject_sets_resubmit_after_30d
// revoke_demotes_active_sites_and_purges：verified org 有 active 據點 → reject 後據點 status "draft"，且 common::cdn mock 收到該 slug 的 purge
// review_notifies_owners：notifications 有 kind 'org.review' 寄給 owner、不寄給 volunteer
// non_staff_forbidden：403 STAFF_ONLY
```

- [ ] **Step 2–4:** FAIL → 實作 → PASS（`--test org_review`）
- [ ] **Step 5:** 在 `mutate.py` 加 `"org-review-for-update"`（把 `FOR UPDATE` 拿掉）和 `"cdn-org-revoke"`（把 purge 拿掉）兩個 mutant，對應的測試分別是 `concurrent_reviews_apply_once`（在本檔新增：兩個 staff 併發 approve／reject 同一 org，最終狀態只能是其中一個，audit 只有一筆生效）和 `revoke_demotes_active_sites_and_purges`。Commit：`feat(api): 組織與許可審核、撤銷時下架據點`

### Task 5: 成員與邀請

**Files:**
- Modify: `apps/api/src/orgs.rs`、`apps/api/src/notify.rs`（kind `org.invite`，只寄給既有使用者）
- Test: `apps/api/tests/org_members.rs`

**Interfaces:**
- Produces：
  - `GET /orgs/{id}/members`：成員可看；回傳 `[{user_id, display_name, role, handle, disabled_at}]`
  - `POST /orgs/{id}/invites {email, role, handle?}`：owner；201 `{id, url}`，`url = {APP_URL}/org/invite/{token}`，token 明文只出現在這次回應；如果 email 對應既有使用者，另外 enqueue `org.invite`
  - `GET /orgs/{id}/invites`、`DELETE /orgs/{id}/invites/{invite_id}`（設定 cancelled_at）
  - `GET /org-invites/{token}`：登入後可看；回傳 `{org_display_name, role, email}`
  - `POST /org-invites/{token}/accept`：登入者必須有和邀請相同的已驗證 email（查 `auth_identities` 的 email provider 或 google 的 email）；否則 403 `INVITE_EMAIL_MISMATCH`，extra 帶 `{email}`。過期或已取消 → 410 `INVITE_EXPIRED`。已是成員 → 409 `ALREADY_MEMBER`。被停用過的成員重新接受邀請時，清掉 `disabled_at` 並更新 role。
  - `PATCH /orgs/{id}/members/{user_id} {role?, handle?, disabled?: bool}`：owner。交易內先 `SELECT user_id FROM org_members WHERE org_id=$1 AND role='owner' AND disabled_at IS NULL ORDER BY user_id FOR UPDATE`；如果這次操作會讓有效 owner 變成 0 人，回 409 `LAST_OWNER`
  - 停用時同交易寫 audit；停用不刪資料

- [ ] **Step 1: 寫失敗測試**

```rust
// invite_accept_flow：owner 邀請 b@x.com 為 coordinator → 使用者 b（已驗證 b@x.com）accept → 200；GET /me 的 orgs 含 role "coordinator"
// invite_email_mismatch：c 帳號 accept b 的邀請 → 403 INVITE_EMAIL_MISMATCH
// invite_expired_or_cancelled → 410
// last_owner_cannot_demote_or_disable_self：唯一的 owner 把自己 PATCH 成 coordinator → 409 LAST_OWNER；disabled:true → 409
// concurrent_mutual_demotion_keeps_one_owner：兩位 owner 用 tokio::join! 同時把對方降為 coordinator → 恰好一個 200、一個 409；最後有效 owner 數 == 1
// disabled_member_loses_access_immediately：停用 coordinator 後，他 GET /orgs/{id} → 404
// only_owner_manages_members：coordinator 呼叫 POST invites → 403 ORG_ROLE_REQUIRED
```

- [ ] **Step 2–4:** FAIL → 實作 → PASS（`--test org_members`）
- [ ] **Step 5:** `mutate.py` 加 `"org-last-owner-for-update"` mutant，對應 `concurrent_mutual_demotion_keeps_one_owner`。Commit：`feat(api): 組織成員邀請、角色與停用`

### Task 6: 據點建立與授權改由 can_manage_wishlist 判斷

**Files:**
- Modify: `apps/api/src/orgs.rs`（建立據點）、`apps/api/src/wishlists.rs`（`load_owned` :202、`owned_item` :1312、`list` :485、`W::json` :196、`create` 拒絕 relief 的訊息保留）、`apps/api/src/claims.rs`、`apps/api/src/contributions.rs`、`apps/api/src/dashboard.rs`、`apps/api/src/uploads.rs`。每一處「以 `owner_id = $user` 判斷擁有權」的地方都要改，用 `grep -n "owner_id *= *\$" apps/api/src/*.rs` 列出來逐一處理
- Test: `apps/api/tests/org_sites.rs`

**Interfaces:**
- Consumes：`can_manage_wishlist`（Task 1）、`require_role`
- Produces：
  - `POST /orgs/{id}/sites {title, description?, event_id, permit_id, site_address, recipient_name, recipient_phone, claim_ttl_hours?}`：coordinator 以上
    - permit 必須屬於這個 org，且狀態為 `approved`（未過期才能發布；草稿階段不檢查期限）
    - event 必須存在、`merged_into IS NULL` 且 `moderation_status='ok'`
    - 建立 wishlist：`type='relief'`、`visibility='public'`、`status='draft'`、`site_status='active'`、`claim_ttl_hours` 預設 48、`owner_id=` 呼叫者
    - 同一交易寫入 `shipping_addresses`（sealed 的 recipient_name／phone／address，地址等於 site_address），讓眾籌能沿用 `capture_and_order`
    - 201，回傳 wishlist JSON
  - `GET /orgs/{id}/sites`：成員可看
  - 擁有權謂詞統一改成 `can_manage_wishlist(owner_id, org_id, $user)`。個人列表 `GET /wishlists` 加上 `AND org_id IS NULL`。
  - `W::json` 的 `org_id`、`event_id`、`permit_id`、`site_address`、`site_status` 改回真實值；relief 清單另外帶 `org: {id, slug, display_name}`
  - `PATCH /wishlists/{id}`：relief 清單不可改 `type`、`visibility`、`org_id`；改 `status='active'` 前，應用層先檢查 org 是 verified（否則 403 `ORG_NOT_VERIFIED`）和 permit 是否有效（否則 409 `PERMIT_INVALID`）。另外開放 `site_status`、`site_address`、`event_id`、`permit_id` 的修改。

- [ ] **Step 1: 寫失敗測試**

```rust
// coordinator_creates_draft_site：201，type "relief"、visibility "public"、status "draft"、claim_ttl_hours 48；shipping_addresses 有該 wishlist
// volunteer_cannot_create_site：403 ORG_ROLE_REQUIRED
// publish_requires_verified_org：org 為 pending 時 PATCH status active → 403 ORG_NOT_VERIFIED；permit 過期 → 409 PERMIT_INVALID；兩者都有效 → 200
// other_coordinator_can_edit_site：同組織的另一位 coordinator 可以 PATCH 據點、POST 品項
// disabled_coordinator_gets_404_on_site_and_item：建立者被停用後，PATCH /wishlists/{id} → 404，PATCH /items/{id} → 404
// org_sites_not_in_personal_list：建立者 GET /wishlists 不含據點
// personal_lists_unchanged：既有 owner 對個人清單仍可 PATCH（回歸測試）
```

- [ ] **Step 2–4:** FAIL → 實作 → PASS。另外要跑整個測試套件 `cargo test --manifest-path apps/api/Cargo.toml`，全部通過才算完成（這是大範圍重構）
- [ ] **Step 5:** Commit：`feat(api): 救災據點建立，擁有權改由 can_manage_wishlist 判斷`

### Task 7: 據點品項（分類、緊急度）與眾籌

**Files:**
- Modify: `apps/api/src/wishlists.rs`（`item_create`、`item_update`、品項 JSON）
- Test: `apps/api/tests/relief_items.rs`

**Interfaces:**
- Produces：品項 body 和 JSON 新增 `category`、`urgency`（relief 必填，非 relief 送了就回 422 pointer `/category`）。眾籌品項照既有規則（`funding_mode:"crowdfund"`、`target_points`、`funding_deadline`）。

- [ ] **Step 1: 寫失敗測試**

```rust
// relief_item_requires_category_urgency_422
// non_relief_item_rejects_category_422
// relief_quantity_claim_works：發布後的據點，使用者 POST /items/{id}/claims（帶 Idempotency-Key）→ 201，expires_at ≈ now+48h
// relief_crowdfund_funds_and_orders_to_site_address：common::grant 點數 → common::pledge 到達目標 → purchase_orders 有一筆，shipping_address_snapshot 解密後 address == site_address；assert_all_reconciled
// paused_site_blocks_new_claims_and_pledges：site_status 'paused' → claim 409 SITE_PAUSED、pledge 409 SITE_PAUSED
```

- [ ] **Step 2–4:** FAIL → 實作 → PASS。`SITE_PAUSED` 的檢查放在 `claims.rs` 的 create 和 `contributions.rs` 的 create，在既有「清單是否可認捐」那段判斷旁邊（讀 `site_status` 的那個 `FOR SHARE` 查詢一併取出，**不新增鎖**）。
- [ ] **Step 5:** Commit：`feat(api): 救災品項分類／緊急度、據點眾籌與暫停`

### Task 8: 災害事件

**Files:**
- Create: `apps/api/src/relief_events.rs`（在 `lib.rs` merge）
- Modify: `apps/api/src/org_admin.rs`（合併、下架）
- Test: `apps/api/tests/relief_events.rs`

**Interfaces:**
- Produces：
  - `const COUNTIES: [&str; 22]`：台灣 22 縣市正式名稱（例如 `"臺北市"`、`"花蓮縣"`），前端 Task 14 用同一份清單
  - `POST /orgs/{id}/events {name, hazard_type, counties, area_note?, occurred_on, description?}`：coordinator 以上，且 org 為 verified（否則 403 `ORG_NOT_VERIFIED`）；201
  - `GET /relief-events/similar?county=&date=`：登入即可；回傳 `counties @> ARRAY[$county]` 且 `occurred_on` 在 ±7 天內、`merged_into IS NULL`、`moderation_status='ok'` 的事件，最多 10 筆
  - `PATCH /relief-events/{id}`：建立事件的 org 中 coordinator 以上的成員；可改 name、description、area_note
  - `POST /admin/relief-events/{id}/merge {into}`
    - 兩個事件依 id 由小到大 `FOR UPDATE`
    - `into` 不可以是自己，也不可以已經被合併
    - 把 `wishlists.event_id` 改到 into，設定 `merged_into`
    - 寫 audit，並 purge 受影響據點的 slug
  - `PATCH /admin/relief-events/{id} {moderation_status, moderation_reason}`：設為 hidden 時，所有掛在這個事件下、`status='active'` 的據點改為 `site_status='paused'` 並 purge

- [ ] **Step 1: 寫失敗測試**

```rust
// unverified_org_cannot_create_event：403 ORG_NOT_VERIFIED
// similar_finds_same_county_within_7_days：花蓮縣 10/20 事件；查 county=花蓮縣、date=10/25 → 找得到；date=11/05 → 找不到；county=臺東縣 → 找不到
// invalid_county_422：pointer "/counties"
// other_org_can_attach_site_but_not_edit_event：B org 用 A 的事件建立據點 → 201；B PATCH 事件 → 403 ORG_ROLE_REQUIRED
// merge_moves_sites_and_marks_merged：merge 後據點的 event_id == into；GET /public/relief-events/{舊 slug} 回 200 且 merged_into == 新 slug（public 端點在 Task 9 實作，這個斷言在 Task 9 的測試補上）
// merge_into_self_or_merged_422
// concurrent_opposite_merges_no_deadlock：A→B 與 B→A 同時執行 → 一個成功、一個 422，沒有 deadlock
// hide_event_pauses_sites
```

- [ ] **Step 2–4:** FAIL → 實作 → PASS
- [ ] **Step 5:** `mutate.py` 加 `"event-merge-lock-order"` mutant（把 ORDER BY id 改成「先鎖來源」），對應 `concurrent_opposite_merges_no_deadlock`。Commit：`feat(api): 災害事件建立、查重、合併與下架`

### Task 9: 公開端點、探索頁排除、許可到期背景工作

**Files:**
- Modify: `apps/api/src/public.rs`、`apps/api/src/relief_events.rs`、`apps/api/src/main.rs`（啟動背景工作）、`apps/api/src/notify.rs`（kind `org.permit_expired` 的 `render_lang`，四語系）
- Test: `apps/api/tests/relief_public.rs`

**Interfaces:**
- Produces（都不讀 cookie，沿用既有公開端點的 Cache-Control）：
  - `GET /public/wishlists/{slug}`：relief 清單另外帶 `org: {slug, display_name, verified: true}`、`event: {slug, name}`、`site_address`、`site_status`
  - `GET /public/wishlists`：加上 `AND type <> 'relief'`
  - `GET /public/relief-events?page=`：條件是 `merged_into IS NULL`、`moderation_status='ok'`，且至少有一個 active＋public 的據點；依 occurred_on DESC 排序
  - `GET /public/relief-events/{slug}`：回傳 `{event, sites: [{slug, title, site_address, site_status, org:{slug, display_name}, needs: {critical, high, medium, low}}], merged_into: null|slug}`。`needs` 是各緊急度「尚未滿足」的品項數：quantity 型看 `qty_claimed < qty_needed`，眾籌型看 funding_status 未完成
  - `GET /public/orgs/{slug}`：只回傳 verified 組織的公開欄位（display_name、intro、website、public_email、public_phone、logo），以及 active 據點列表
  - `pub fn spawn_permit_expiry(pool)`：每小時執行一次。把 `type='relief' AND site_status='active'`、而綁定的 permit 已過 `valid_to` 的據點改為 `paused`，enqueue `org.permit_expired` 通知給 owner，並 purge 這些 slug

- [ ] **Step 1: 寫失敗測試**

```rust
// public_site_includes_org_and_event
// explore_excludes_relief
// events_list_only_with_active_public_sites
// event_detail_needs_summary：2 個 critical 品項，其中 1 個已認滿 → needs.critical == 1
// merged_event_returns_merged_into（Task 8 留下的斷言）
// org_public_page_only_verified：pending org → 404
// permit_expiry_pauses_sites_and_purges：permit valid_to = 昨天 → 呼叫一次 tick → site_status "paused"，cdn mock 收到 purge，notifications 有 org.permit_expired
// public_endpoints_ignore_cookie：帶 ws_session cookie 和不帶的回應 body 相同
```

- [ ] **Step 2–4:** FAIL → 實作 → PASS。背景工作照 `ratelimit::spawn_cleanup` 的寫法，暴露 `pub async fn permit_expiry_tick(pool) -> Result<usize, sqlx::Error>` 給測試呼叫
- [ ] **Step 5:** `mutate.py` 加 `"cdn-permit-expiry"` mutant。Commit：`feat(api): 救災公開端點、探索頁排除、許可到期暫停據點`

### Task 10: 實收登記

**Files:**
- Modify: `apps/api/src/orgs.rs`
- Test: `apps/api/tests/relief_receipts.rs`

**Interfaces:**
- Produces：
  - `POST /orgs/{id}/receipts {item_id, qty, note?}`：任何未停用的成員都可以登記（volunteer 也行）；需要 `Idempotency-Key`
    - 品項所屬的清單必須是這個 org 的據點，且 `site_status <> 'closed'`
    - 寫入 `receipts` 並 audit；201
  - `GET /orgs/{id}/sites/{wishlist_id}/receipts`：成員可看；回傳每個品項的實收合計和明細
  - 品項 JSON（管理端）帶 `qty_received`（實收合計）

- [ ] **Step 1: 寫失敗測試**

```rust
// volunteer_records_receipt：201；GET 合計 == qty
// closed_site_rejects：409 SITE_CLOSED
// item_of_other_org_404
// idempotent_replay：同一個 key 送兩次 → 只有一筆，第二次回應帶 Idempotency-Replayed
```

- [ ] **Step 2–4:** FAIL → 實作 → PASS
- [ ] **Step 5:** Commit：`feat(api): 據點實收登記`

### Task 11: 前端基礎 — /org 介紹頁、身分切換、組織 API

**Files:**
- Create: `apps/web/app/pages/org/index.vue`（SSR，訪客可看）、`apps/web/app/composables/useOrgMode.ts`、`apps/web/i18n/locales/*/org.json`（四語系；i18n 設定若需要逐檔註冊就一併加上）
- Modify: `apps/web/app/components/creator/UserMenu.vue`（身分選單）、`apps/web/app/components/creator/Header.vue`（組織模式標記）、`apps/web/app/pages/login.vue`（「公益組織？從這裡申請」連到 `/org`）、`apps/web/nuxt.config.ts`（`routeRules` 加 `/org/apply`、`/org/invite/**`、`/orgs/**`）
- Test: `apps/web/tests/orgMode.test.ts`

**Interfaces:**
- Produces：`useOrgMode()` → `{ current: Ref<string|null> /* org_id；null 表示個人 */, orgs: ComputedRef<MeOrg[]>, switchTo(id: string|null): void }`
  - 狀態存 `localStorage['wishly-org-mode']`（try/catch）
  - 如果存的 id 不在 `/me` 的 `orgs` 裡就回到 null
  - `switchTo` 會導到 `/orgs/{id}`，切回個人時導到 `/dashboard`
- `MeOrg = { id: string, name: string, role: 'owner'|'coordinator'|'volunteer', verification_status: string }`

- [ ] **Step 1:** 寫 `orgMode.test.ts`：stale id 會回退成 null；`switchTo` 會寫入 storage；localStorage 丟例外時不會壞
- [ ] **Step 2:** `cd apps/web && npx vitest run tests/orgMode.test.ts`，FAIL
- [ ] **Step 3:** 實作。`/org` 介紹頁的內容照草圖 `org-entry.html` A 案：標題、四個賣點、申請前準備的文件、CTA。CTA 未登入時導到 `/login?redirect=/org/apply`，已登入則直接到 `/org/apply`。
- [ ] **Step 4:** `npm run typecheck && npx vitest run`，PASS
- [ ] **Step 5:** Commit：`feat(web): 公益組織介紹頁與身分切換`

### Task 12: 前端 — 申請表與狀態頁

**Files:**
- Create: `apps/web/app/pages/org/apply.vue`、`apps/web/app/utils/org.ts`
- Test: `apps/web/tests/org.test.ts`

**Interfaces:**
- Produces（`utils/org.ts`）：
  - `taxIdOk(s: string): boolean`：和後端演算法相同，測試案例共用 `"04595257"`
  - `ORG_TYPES`
  - `stepOf(org): 1|2|3|4`：依缺漏欄位決定從第幾步繼續
  - `editable(org, field): boolean`：needs_changes 時只開放 `flagged_fields` 裡的欄位
- 頁面行為：
  - 沒有草稿時 `POST /orgs`；有草稿就沿用 `/me` 的 orgs 裡 draft／needs_changes／pending／rejected 的那一個
  - 欄位 blur 時 `PATCH`（草稿自動保存）
  - 第 3 步直接 `$fetch` 上傳原始檔，要帶 `nativeHeaders()` 和 `credentials:'include'`
  - 狀態區塊依 `verification_status` 切換四種畫面（草圖 `org-apply-form.html`）；verified 時顯示「切換到組織身分」→ `switchTo(id)`

- [ ] **Step 1:** 寫 `org.test.ts`：`taxIdOk` 的真假案例；`editable` 在 needs_changes 狀態下的判斷；`stepOf` 對缺 tax_id 的草稿回 1、對缺文件的草稿回 3
- [ ] **Step 2–4:** FAIL → 實作 → `npm run typecheck && npx vitest run` PASS
- [ ] **Step 5:** Commit：`feat(web): 組織申請表四步驟與審核狀態`

### Task 13: 前端 — 組織後台與成員

**Files:**
- Create: `apps/web/app/pages/orgs/[id]/index.vue`（總覽：事件、據點列表，以及「新增據點」「新增事件」按鈕，依角色顯示）、`apps/web/app/pages/orgs/[id]/members.vue`、`apps/web/app/pages/org/invite/[token].vue`

**Interfaces:**
- Consumes：Task 5 的 API、`useOrgMode`
- 行為：
  - 邀請建立後顯示連結，附複製按鈕和 QR（沿用 `creator/Qr.vue`）
  - 接受邀請時遇到 `INVITE_EMAIL_MISMATCH`，顯示 spec §10-7 的文案（帶入 email）
  - 頁面載入時呼叫 `switchTo` 對齊路由裡的 org id

- [ ] **Step 1:** 寫 e2e `apps/web/e2e/org-members.spec.ts`：owner 邀請 → 用 Mailpit 註冊受邀的 email → 開邀請連結 → 接受 → 選單出現該組織
- [ ] **Step 2–4:** FAIL → 實作 → PASS（`npx playwright test e2e/org-members.spec.ts`）
- [ ] **Step 5:** Commit：`feat(web): 組織後台總覽、成員與邀請`

### Task 14: 前端 — 建立事件與據點、據點品項

**Files:**
- Create: `apps/web/app/pages/orgs/[id]/events/new.vue`、`apps/web/app/pages/orgs/[id]/sites/new.vue`、`apps/web/app/utils/relief.ts`
- Modify: `apps/web/app/components/creator/ItemSheet.vue`（relief 清單時顯示 category、urgency，兩欄必填）、`apps/web/app/pages/lists/[id]/*`（relief 清單隱藏可見性選擇和收件資訊表單，改顯示據點地址、`site_status` 切換、實收登記入口）

**Interfaces:**
- Produces（`utils/relief.ts`）：`COUNTIES`（和後端相同的 22 筆）、`HAZARDS`、`CATEGORIES`、`URGENCIES`、`mapsUrl(address: string): string`（回傳 `https://www.google.com/maps/search/?api=1&query=` 加上 encodeURIComponent 後的地址）
- 建立事件時，選完縣市和日期就呼叫 `/relief-events/similar`。有結果時先列出「是不是這個？」並提供「用這個事件建立據點」，使用者確認都不是才顯示建立表單。

- [ ] **Step 1:** vitest `tests/relief.test.ts`：`COUNTIES.length === 22`；`mapsUrl` 有正確編碼。e2e `e2e/org-relief.spec.ts`：用 SQL fixture 建好 verified org 和 approved permit → 建立事件（查重有命中時選既有事件）→ 建立據點 → 加入 critical 品項 → 發布 → 公開頁看得到
- [ ] **Step 2–4:** FAIL → 實作 → PASS
- [ ] **Step 5:** Commit：`feat(web): 建立災害事件與據點、救災品項欄位`

### Task 15: 前端 — 後台審核頁

**Files:**
- Create: `apps/web/app/pages/admin/orgs.vue`、`apps/web/app/pages/admin/relief-events.vue`
- Modify: `apps/web/app/pages/admin/index.vue`（加入口）

**Interfaces:**
- Consumes：Task 4 和 Task 8 的 admin API
- 行為：
  - 列表預設篩選 `status=pending`
  - 詳情顯示所有欄位、文件下載連結（直接 `$fetch` blob，要帶 `nativeHeaders()`）、permits 和各自的審核按鈕
  - 「需補件」用勾選框選欄位，選項是可編輯欄位加上「文件」「許可」
  - 撤銷（verified → reject）要二次確認，沿用 `creator/Confirm.vue`

- [ ] **Step 1:** e2e `e2e/admin-orgs.spec.ts`：staff 打開 pending 申請 → 核准 permit → 核准組織 → 申請人的狀態頁顯示已通過
- [ ] **Step 2–4:** FAIL → 實作 → PASS
- [ ] **Step 5:** Commit：`feat(web): 後台組織審核與事件管理`

### Task 16: 前端 — 公開頁

**Files:**
- Create: `apps/web/app/pages/e/index.vue`、`apps/web/app/pages/e/[slug].vue`、`apps/web/app/pages/o/[slug].vue`（三頁都是 SSR，用 `useAsyncData` + `$fetch`，不帶 cookie，比照 `brands/index.vue`）
- Modify: `apps/web/app/pages/s/[slug].vue`（relief 清單顯示組織名稱、「已驗證組織」徽章、所屬事件連結、地址和 `mapsUrl`；`paused` 時顯示暫停橫幅並停用認捐按鈕）、`apps/web/app/pages/explore.vue`（頂端加「災害救援」橫幅連到 `/e`）、`apps/web/mocks/server.mjs`（加一個 relief slug `relief` 和事件 `quake`，方便不起後端開發）

**Interfaces:**
- `/e/[slug]` 回應裡 `merged_into` 不為 null 時，`navigateTo('/e/' + merged_into, { redirectCode: 301 })`
- 每個據點卡片顯示 `needs.critical` 和 `needs.high` 的數字（critical 用紅色）

- [ ] **Step 1:** e2e `e2e/relief-public.spec.ts`：用 mock server 或 SQL fixture 驗證三件事 — `/e` 列表有事件；`/e/quake` 有據點和缺口數字；打開舊 slug 會被導到合併後的事件
- [ ] **Step 2–4:** FAIL → 實作 → `npm run typecheck && npm test && npm run build` PASS
- [ ] **Step 5:** Commit：`feat(web): 災害事件與組織公開頁、分享頁顯示組織資訊`

### Task 17: 文件同步

**Files:**
- Modify: `docs/04-data-schema-api-contract.html`（organizations／events／sites 的實際 schema、新端點、錯誤碼 `ORG_NOT_VERIFIED`、`ORG_ROLE_REQUIRED`、`TAX_ID_TAKEN`、`TAX_ID_CHECKSUM`、`FIELD_LOCKED`、`RESUBMIT_TOO_EARLY`、`ORG_DRAFT_EXISTS`、`PERMIT_REQUIRED`、`PERMIT_INVALID`、`INVITE_EMAIL_MISMATCH`、`INVITE_EXPIRED`、`ALREADY_MEMBER`、`LAST_OWNER`、`SITE_PAUSED`、`SITE_CLOSED`）、`docs/01-prd.html`（新增決策：relief 開放點數眾籌，推翻原本「僅數量型」；災害事件；用 D 編號接續現有最大號）、`CLAUDE.md`（架構段落加一行 orgs／relief_events 的鎖序和 `can_manage_wishlist`）、`README.md`（如果有新環境變數或背景工作）

- [ ] **Step 1:** 依實作內容更新上述文件
- [ ] **Step 2:** `scripts/mutate.sh all`，全部 killed，且還原後 SHA 一致
- [ ] **Step 3:** Commit：`docs: 公益組織救災第一段的契約、PRD 決策與架構說明`
