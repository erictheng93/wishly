# 公益組織（org）註冊、審核與救災清單：設計

日期：2026-10-27　狀態：草案，待產品負責人審閱
畫面草圖：`.superpowers/brainstorm/46627-1793081498/content/`（`org-entry.html`、`org-apply-form.html`，本機檔，不進 git）

## 1. 意圖

Wishly 之後要支援非營利／公益組織（下稱 org），重點是救災。災害發生時，救災團體到 Wishly 註冊組織、交給平台方審核；通過後，他們可以針對某場災害開出需求清單。清單一律公開，任何使用者都能瀏覽、認捐物資或用點數眾籌。

為什麼重要：救災是信任問題。平台方審核資格、錢不經 org 之手（眾籌由平台代購物資），是 Wishly 能給捐助者的保證。

## 2. 和既有規格的關係（整合評估結論）

PRD／流程／契約裡**早已規劃 P2-D relief**，但**完全沒有實作**：`auth.rs` 的 `/me` 回 `"orgs": []`，`wishlists.rs` 拒絕 `type=relief`，DB 只有 `wishlist_type` 枚舉裡的 `relief` 值，沒有 `organizations` 表。本設計**以既有 P2-D 規格為底**，只寫差異。既有規格出處：

- `docs/01-prd.html`：US-13～16、US-44、D43、D52、8.7 法規阻斷項
- `docs/02-master-user-flow.html`：F10、F10-0、F10-a
- `docs/04-data-schema-api-contract.html`：`organizations`、`org_members`、`org_role`、`wishlists.org_id/location/site_status`、`receipts`、`wishlists_check_org_verified` trigger

### 2.1 沿用（不改）

| 項目 | 既有規格 |
|---|---|
| 組織實體 | `organizations` + `org_members(org_id, user_id, role)`，建立組織與第一位 owner 在同一交易 |
| 角色 | `owner` / `coordinator` / `volunteer`。介面顯示為 管理者 / 工作人員 / 志工 |
| 角色權限 | owner：成員、組織資料、送審。coordinator：建立／編輯清單與品項、取消捐助、標記已送達。volunteer：登記實收、看儀表板 |
| 最後一位 owner | 不可降級或移除（`FOR UPDATE` 檢查） |
| 驗證狀態 | `verification_status`：`pending` / `verified` / `rejected`；驗證文件上傳到 R2 私有路徑，只給該 org 成員和 staff 用短效簽名網址讀 |
| 未驗證不得公開 | trigger `wishlists_check_org_verified` → 403 `ORG_NOT_VERIFIED`。撤銷驗證時，同一交易把 public 據點降為 draft，並 `cdn::purge_wishlists` |
| 清單＝據點 | relief 清單就是 `type=relief` 的 wishlist，帶 PostGIS 座標、地址、`site_status`（active / paused / closed），強制 public、不顯示捐助者名稱、不能開驚喜模式，認捐預設 48 小時逾期 |
| 物資認捐 | 沿用 `claims.rs`（要登入、需 `Idempotency-Key`、防超賣鎖序不變） |
| 實收 | `receipts` 表 = 志工登記「實際收到幾件」 |
| 不收災民個資 | 只記錄據點層級的需求 |
| 後台 | `/admin/orgs?verification_status=`、`PATCH /admin/orgs/{id}/verification`，掛在現有 `/admin` 殼，寫 `admin::audit` |
| 收費 | relief 對合格組織永久免費 |

### 2.2 本設計新增或推翻的決定（產品負責人 2026-10-27 決定）

1. **org 帳號與個人帳號的關係**：同一個人用同一個 email 登入，在「個人」和「組織」之間切換身分。不另開組織專用的登入帳號，也不發 `xxx@wishly.tw` 這種信箱（理由：現有登入靠 email 驗證和重設密碼，假信箱收不到信；共用編號帳號會讓操作紀錄對不到人；`@wishly.tw` 容易被冒用成官方帳號）。「隔離」指資料和權責隔離（見 §4）。
2. **一個人可以屬於多個 org。**
3. **成員用 email 邀請**：既有規格只有「以 email 找已存在的 user」，這次改為邀請制（`org_invites`，§5.2），對方還沒有帳號時可以先註冊再加入。
4. **申請時先送審**：org 要填完申請表（4 步）、上傳文件、送出審核。既有規格是「任何登入者都能建立 org，驗證另外申請」，這次改成「建立組織＝送出申請」。
5. **災害事件（新概念）**：由 org 建立，**多個 org 共用同一個事件**，平台方只做事後的合併和下架。清單（據點）掛在事件底下。
6. **relief 也開放點數眾籌**：**推翻**既有規格「relief 仍只用數量捐助，不接點數」（PRD 1212 行、契約 relief 品項 trigger）。眾籌沿用 P2-A：品項集滿後，平台建立採購單代購，寄到據點，錢不經 org。
7. **捐款收據（新）**：捐助者可以選擇要不要收據；收據由 org 開立，可以用來報稅抵扣。**命名注意**：既有的 `receipts` 是「實收」，新表一律叫 `donation_receipts`，介面也要避免兩者混淆。
8. **入口**：專屬介紹頁 `/org`。

## 3. 目標／非目標／反目標

**目標**
- 救災團體從收到連結到送出申請，大約 15 分鐘內完成；平台審核以 3 個工作天為目標。
- 審核通過後，org 可以馬上建立或加入災害事件、發布據點清單。
- 每個操作都對得到具體的人（`audit_logs` 記錄 actor user_id + org_id）。

**非目標（本版不做）**
- 平台方建立災害事件
- 組織之間互相轉移物資或調度
- 組織的金流（錢不給 org）
- 離線登記（US-16 維持原排程，不在本設計範圍）
- 自動化的勸募許可查詢串接（人工審核看文件）

**反目標（就算功能能動也算失敗）**
- 未經審核的組織內容出現在公開頁或探索頁
- 捐助者的身分證字號被志工或其他 org 看到
- 個人的好友、私人清單、錢包資料出現在組織模式裡；或反過來，組織內容出現在個人頁
- 平台收的眾籌款項以現金形式流到 org

## 4. 身分與切換

- 登入照舊（`session::CurrentUser`）。「目前操作身分」是前端狀態：個人，或某個 org_id。組織端點一律把 org_id 放在路徑上（`/orgs/{id}/...`），後端每次都檢查 `org_members` 的角色，**不依賴 session 裡記錄的「目前身分」**。所以就算切換狀態錯了，也只會影響畫面，不會越權。
- 右上角是身分選單：個人／各組織（附角色）。組織模式下，頁首顯示組織名稱和紅色「組織模式」標記。
- 組織模式的後台只有：事件、清單（據點）、成員、組織資料、捐款收據匯出。個人的好友、錢包等選單隱藏。
- 被停用或移除的成員：下一個請求就失去權限（每次查 `org_members`），操作紀錄保留。
- `GET /me` 的 `orgs[]` 依既有契約回傳 `{id, name, role}`，多加 `verification_status`。

## 5. 流程

### 5.1 申請與審核

```
/org 介紹頁 →（未登入）現有 /login 或 /register?redirect=/org/apply
→ /org/apply 申請表 4 步（草稿自動保存）→ 送出
→ 狀態：審核中 →（需補件 ⇄ 重送）→ 已通過 / 未通過
```

- `/org` 是訪客可看的 SSR 頁，說明服務、申請前要準備的文件，CTA 是「申請成為公益組織」。登入頁也加一行連結。
- **申請表**
  1. **基本資料**：組織全名（與立案證書一致）、對外顯示名稱、組織類型（社團法人／財團法人／宗教團體／其他立案團體）、統一編號（8 碼，即時檢查格式和檢查碼）、立案字號與主管機關、官網或粉專（選填）、簡介、Logo。
  2. **負責人與聯絡**：負責人姓名；聯絡窗口姓名、職稱、手機（只給審核用，不公開）；對外公開的 email 和電話。
  3. **證明文件**（PDF / JPG / PNG，每個檔案 ≤10MB，私有儲存）：立案證書（必填）；勸募許可函，含字號和有效期間（必填）；可開立抵稅收據的資格證明（選填，沒有的話該 org 不提供「需要收據」選項）。
  4. **確認送出**：預覽資料，勾選「資料真實、同意組織條款」。
- **草稿**：DB 裡是 `organizations` 一列，`verification_status='draft'`。這是新增的狀態值；申請送出後轉為 `pending`。
- **審核中可以撤回**（回到 draft）。
- **需補件**：新狀態 `needs_changes`，附審核員意見，以及被點名的欄位清單。org 只能修改這些欄位，改完重送回 `pending`。
- **未通過**：`rejected` + 原因，30 天後才能重新送審（同一個組織列，不另建新列）。
- **通過**：`verified`，email 通知，狀態頁出現「切換到組織身分」按鈕。
- **審核期間可以先建立草稿清單**，但不能發布（沿用 D52）。

> 我的判斷：你說「審核通過後才能開清單」。我理解為「通過後才能**公開**」，所以允許先打草稿，這樣災害發生時可以和審核同步準備。如果你要的是審核前完全不能建清單，改成擋住 `POST /orgs/{id}/sites` 就好。

### 5.2 成員邀請

- owner 在「成員」頁輸入 email、選擇角色、填組織內的顯示代號（選填，例如 `worker1`，只在組織內顯示，不是 email）。
- 系統建立 `org_invites`：存權杖 hash，7 天有效，透過 `notify` outbox 寄信。對方點連結 `/org/invite/{token}`，登入或註冊後加入組織。邀請綁定 email：登入帳號的已驗證 email 必須和邀請的 email 相同。
- owner 可以改角色、停用成員；最後一位 owner 不能降級或移除。

### 5.3 災害事件

- org（coordinator 以上，且組織已驗證）建立事件時要填：名稱、災害類型（地震／颱風／水災／火災／其他）、受災地區（縣市＋鄉鎮，可多選，另存一個中心座標）、發生日期。
- **建立前查重**：同一縣市、發生日期前後 7 天內的事件，先列出來問「是不是這個？」，優先加入既有事件。
- **事件沒有 owner 欄位**，只記錄建立者 org。任何已驗證的 org 都可以把據點掛上去。建立者 org 可以修改事件名稱和描述。
- **平台方後台**：合併事件（把 A 的據點搬到 B，A 改為 `merged_into=B` 並導向）、下架不實事件。
- **公開頁 `/e/{slug}`**：事件資訊、所有 org 的據點列表和地圖、缺口彙總。據點頁顯示所屬事件和 org（附「已驗證組織」徽章）。

### 5.4 據點清單

- 沿用 `POST /orgs/{id}/sites`，新增必填的 `event_id` 和 `permit_id`。據點要綁定 org 的一張**有效勸募許可**；許可到期後，背景工作把據點 `site_status` 轉為 `paused`，並通知 org。
- **寄送地址**：據點的地址就是物資收件點，公開顯示（這是組織的集散站，不是個人地址，所以不用 `sealed`）。
- **品項**：既有的 `category` / `urgency` 加上 `funding_mode`。既有 trigger 限定 relief 只能用 quantity，這次放寬為 quantity 或 crowdfund（§2.2-6）。

### 5.5 捐款收據

- 認捐或眾籌時，如果 org 有收據資格，就顯示「需要捐款收據」勾選框。勾選後填：收據抬頭（姓名或公司）、身分證字號或統編（要抵稅才填）、寄送 email。
- 存在 `donation_receipts`：敏感欄位用 `sealed.rs` 加密，連到 claim 或 contribution。
- **org 匯出**：只有 owner / coordinator 可以下載 CSV（解密），格式以能配合財政部捐贈資料上傳為目標，實際欄位交給 builder 查證。每次匯出都寫 audit。volunteer 看不到。
- **收據金額**：物資用品項單價 × 實收數量；眾籌用實際投入的點數。這兩點都等律師確認（§7）。
- 帳號刪除（D40）時，收據資料依法保存的部分走假名化，處理方式比照發票。

## 6. 資料與 API 變更（相對既有契約）

**Migration（新檔，依序累加）**：落實既有契約裡的 `organizations` / `org_members` / `wishlists` relief 欄位 / `receipts` / trigger，另外加上：
- `organizations`：申請欄位（統編、立案字號、主管機關、類型、負責人、聯絡窗口、公開聯絡方式、Logo）；`verification_status` 加 `draft`、`needs_changes`；審核意見、被點名欄位（`jsonb`）、`resubmit_after`；`can_issue_tax_receipt`。
- `org_documents`：org_id、kind（`registration` / `permit` / `tax_receipt_eligibility`）、檔案 key、上傳者。
- `org_permits`：org_id、字號、有效起訖、審核狀態。
- `org_members`：加 `handle`（組織內顯示代號）、`disabled_at`。
- `org_invites`：org_id、email、role、handle、token_hash、expires_at、accepted_at。
- `relief_events`：slug、name、hazard_type、regions、center（geography）、occurred_on、created_by_org、merged_into、moderation_status。
- `wishlists`：加 `event_id`、`permit_id`（relief 必填，用 CHECK）。
- `donation_receipts`：claim_id 或 contribution_id（兩者擇一）、org_id、sealed 欄位、建立時間。
- 放寬 relief 品項 trigger，允許 crowdfund。

**API（新增或變更，路徑掛在 `/api/v1`，每個模組自己的 `routes()`）**
- 新模組 `orgs.rs`：申請草稿 CRUD、送審、撤回、文件上傳（`uploads.rs` 新增 `purpose=org_document`）、成員與邀請、許可。
- 新模組 `events.rs`：建立、查重（`GET /events/similar?region=&date=`）、編輯、公開讀取。
- 公開讀取 `GET /public/events/{slug}`、`GET /public/relief/*`：照既有規則不讀 cookie，可以讓 CDN 快取。
- `admin.rs`：組織審核（通過／需補件／退件）、許可審核、事件合併／下架。全部限 staff、寫 audit。
- `claims.rs` / `contributions.rs`：可選帶 `donation_receipt`，和業務寫入在同一交易。**鎖序不變**；收據列在最後才寫。
- `GET /me`：`orgs[]` 加 `verification_status`。

**前端**
- 新路由：`/org`（SSR，訪客）、`/e/[slug]`（SSR）。
- 創建者路由（要加進 `nuxt.config.ts` 的 `routeRules` `ssr:false`）：`/org/apply`、`/org/invite/[token]`、`/orgs/[id]/...`（事件、據點、成員、收據）、`/admin/orgs`、`/admin/events`。
- 身分選單元件放在 `CreatorHeader`。
- 原生殼直接 `$fetch` 的請求要帶 `nativeHeaders()`。

## 7. 上線前阻斷項（需律師／會計師，不是工程決定）

1. **勸募許可**：org 在 Wishly 上公開募集物資或點數，是否要用勸募許可？許可的範圍（地區、期間、用途）和清單要怎麼對應？
2. **平台代收代購**：點數眾籌由平台代收款項、代購物資交給 org，平台是否構成「代為勸募」，或需要其他金流資格？這會合併進既有的 8.7 阻斷項。
3. **捐款收據的捐贈人是誰**：眾籌的錢是捐助者儲值給平台的，收據要開給捐助者還是平台？物資捐贈的金額怎麼認定？
4. **身分證字號的蒐集告知**：個資法告知事項、保存期限。

工程上分兩段：**第一段**是 org 申請／審核、成員、事件、據點、物資認捐、實收，以及點數眾籌（產品負責人 2026-10-27 決定納入；§7-2、§7-3 仍是正式上線前的阻斷項，不阻擋開發）；**第二段**是捐款收據，等 §7-3、§7-4 確認後再做。

## 8. 交給 builder 決定的事

- 統編檢查碼的演算法、檔案類型白名單、草稿自動保存的頻率
- 查重的實際條件（地理距離、日期窗口）
- 捐贈資料匯出 CSV 的實際欄位（查財政部格式）
- 介面文案，以及「組織模式」標記的樣式（依 `assets/tokens.css` 紅白實驗室主題）
- 併發相關的 SQL（成員最後一位 owner 檢查、事件合併）要在 `scripts/mutate.py` 補 mutant

## 9. 我做的判斷（請審閱時特別看）

- 審核 3 個工作天、退件後 30 天才能重送、檔案上限 10MB、邀請 7 天有效
- 審核期間可以先建立草稿清單（§5.1）
- 收據資格是選填，沒有資格就不提供收據選項
- 據點地址公開、不加密
- 事件建立後由建立者 org 編輯，其他 org 只能掛據點
- 分兩段上線（§7）
- §10 全部

## 10. 審閱後補充的決定（皆為我的判斷）

1. **勸募許可**：
   - 申請表第 3 步填寫許可字號和有效起訖、上傳許可函，送出時建立一筆 `org_permits`，和組織一起審核。組織要通過驗證，至少要有一張核准的許可。
   - 之後可以在組織設定裡新增許可（一個組織可以有多張），每張各自送審。
   - 據點只能選「已核准且未過期」的許可。許可的地區和用途是否涵蓋這個事件，由審核員人工判斷，系統不自動比對（等 §7-1 的法律意見再看要不要自動化）。
2. **據點的擁有與權限**：`wishlists.owner_id` 照舊存建立者（NOT NULL），但 relief 據點的所有權限**一律看 `org_members` 的角色和 `disabled_at`，不看 `owner_id`**。在 `access.rs` 和 wishlists 的擁有者檢查裡加上 relief 分支。個人的 dashboard 查詢要排除 `org_id IS NOT NULL`。
3. **第一段的範圍**：
   - migration 只建第一段要用的表（含放寬 relief 品項 trigger 允許 crowdfund）。`donation_receipts`、`can_issue_tax_receipt` 留到第二段。
   - 申請表第 3 步的「收據資格證明」欄位延到第二段才開放。不另外做 feature flag。
4. **統編唯一**：
   - 狀態為 `pending`、`needs_changes`、`verified` 的組織，統編不能重複（partial unique index），撞到時回 409 並提示聯絡平台。分會或分支機構的情況由 staff 人工處理。
   - 每個使用者同時最多 1 份 draft 申請。
5. **公開曝光**：relief 據點**不列入** `GET /public/wishlists` 探索頁，只透過事件頁 `/e/{slug}`、新的事件列表 `/e`（列出進行中的事件）和 `/public/relief/*` 曝光。探索頁頂端加一條連到 `/e` 的橫幅。撤銷驗證和事件下架時，對受影響的據點呼叫 `cdn::purge_wishlists`，另外清除事件頁的快取。
6. **事後停權**：staff 可以把已驗證的組織直接改為 `rejected` 並附上原因（沿用既有的撤銷流程：據點降為 draft、purge CDN）。不另設 suspended 狀態。`needs_changes` 只用在送審階段。審核歷程記在 `audit_logs`。
7. **邀請 email 不符**：接受邀請時，如果登入帳號沒有和邀請相同的已驗證 email，顯示「請用 x@… 登入，或請管理者改寄到你的 email」。owner 可以取消邀請再重新寄。
