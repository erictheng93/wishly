# 物資提議：捐助者提議清單外的物資，由組織決定收不收

日期：2026-11-04

狀態：**待試點驗證後才實作**。PRD §3.3 要求救災需求先由合作夥伴試點驗證，不可自行想像需求。試點訪談時先問組織兩件事：
1. 有沒有遇過捐助者要送清單外的東西？當時怎麼處理？
2. 如果要在系統裡審核這類提議，多久內能回覆？

只要有一個試點組織表示需要、也有人力處理，就照本文件實作。

## 目的

捐助者手上常有據點清單沒列的物資。直接送到據點，容易造成災後物資堆積（PRD「救災捐贈者」Persona 的痛點：怕堆積無用物資）。讓捐助者先提議，**組織同意才收**：組織保有決定權，捐助者也能確定東西有人要。

## 使用者決定

- 只開放給公益組織的救災據點（`type='relief'`），個人清單不做。
- 由開立據點的組織審核與回覆：組織的 owner 與 coordinator 可以處理，用 `can_manage_wishlist` 判斷，與管理據點品項的權限相同；volunteer 不行。
- 組織接受後，**同一筆交易**內新增品項，並替提議者建立認領。
- 組織審核時**完全看不到提議者是誰**（與 D58 一致）。
- 不做「只開開關、不做審核」的第零步。據點說明（`description`）加上組織聯絡方式已能涵蓋這個用途；據點頁顯示聯絡方式與勸募許可已在 #152 完成。

## 名詞

- **提議**：捐助者對某據點提出的一筆清單外物資。
- **提議者**：送出提議的登入會員。
- **審核者**：據點所屬組織的 owner / coordinator。

## 非目標

- 個人清單、一般心願清單的提議（會破壞驚喜模式並帶來人情壓力，見 PRD US-08）。
- 眾籌品項的提議。提議只針對實體物資，數量型品項。
- 雙向對話。組織只能「接受」或「婉拒並附理由」，不做留言串（避免變成私訊管道）。
- 用名稱比對自動判斷是否與現有品項重複。改為在提議表單上列出據點目前的需求，並提供婉拒理由「已有此品項」。
- 由組織自訂處理期限。第一版固定 24 小時，試點後再看是否需要調整。

## 不可出現的結果

- 審核者在任何畫面、API、通知、稽核紀錄看到提議者身分。
- 接受後提議者沒拿到認領，被別人先認走。
- 據點暫停、關閉或組織被撤銷驗證後，還能接受舊提議。
- 同一筆提議被接受兩次（重複建立品項或認領）。
- 組織透過封鎖或封鎖名單得知提議者身分。
- 被封鎖的提議者從任何回應得知自己被封鎖。

## 流程

1. **據點設定**：據點新增欄位 `accepts_proposals boolean NOT NULL DEFAULT false`。組織在據點編輯頁開啟後，分享頁才出現「我想送清單外的物資」入口。
2. **提議**（捐助者，須登入，D50）：
   - 表單上方列出據點目前未滿足的需求，提醒先看有沒有現成品項。
   - 欄位：品名（必填，≤ 80 字）、數量（必填，正整數）、物資分類（必填，沿用 `category`：water、food、medical、hygiene、shelter、clothing、baby、other）、說明（選填，≤ 500 字）、預計送達方式（選填，≤ 200 字）。
   - 急迫度不由提議者填（D62 要求品項有 `urgency`，接受時由組織填）。
3. **審核**：審核者在組織後台的據點頁看到待審提議清單，只顯示上述欄位與送出時間。
   - **接受**：填急迫度（必填），可調整品名、數量與分類。送出後在同一交易內：
     1. 新增品項，數量為接受的數量。
     2. 替提議者建立認領，狀態為 `reserved`，數量等於品項數量。
     3. 依據點的 `claim_ttl_hours`（預設 48）計算逾期時間，從接受當下開始計時。
     4. 提議改為 `accepted`，並記下新品項的 id。
   - **婉拒**：選理由（`already_listed` 已有此品項、`not_needed` 目前不需要、`cannot_store` 無法存放或處理、`other` 其他），可附 ≤ 200 字說明。
4. **結果通知**：提議者收到接受或婉拒的通知。接受的通知要寫出逾期時間，並連到該認領。
5. **自動過期**：提議送出 24 小時內沒有處理，就轉為 `expired`，提議者收到通知（理由視為「組織未於期限內回覆」）。審核清單上每筆提議都顯示剩餘時間。

### 為什麼是 24 小時

- 救災需求變化以天計，太晚回覆的接受判斷可能已經過時。
- 72 小時審核加 48 小時認領期限，從提議到送達最長 5 天，捐助者早就送去別處了。
- 被婉拒或過期的捐助者越早知道，越能轉向其他據點（US-15 的精神）。
- 據點通常每天開一次晨會，24 小時保證至少經過一輪。晚上送出的提議要等到隔天 09:00 才寄 Email，組織仍有超過 12 小時的白天時間處理。

## 資料

新表 `item_proposals`：

| 欄位 | 說明 |
|---|---|
| `id` | uuid v7 |
| `wishlist_id` | 據點，外鍵 |
| `user_id` | 提議者。**刻意不設外鍵**（理由同 0050：避免外鍵檢查鎖到 `users`，與帳號刪除的鎖序相反） |
| `name`、`qty`、`category`、`detail`、`delivery_note` | 提議內容 |
| `status` | `pending` / `accepted` / `declined` / `expired` / `withdrawn` |
| `decline_reason`、`decline_note` | 婉拒時填 |
| `item_id` | 接受後建立的品項 |
| `decided_at`、`created_at` | |

- 部分索引：`(wishlist_id) WHERE status = 'pending'`，供審核清單用。
- 提議者可以撤回 `pending` 的提議，狀態改為 `withdrawn`。

## API

| 方法與路徑 | 說明 |
|---|---|
| `POST /wishlists/{id}/proposals` | 提議者送出。須登入、須帶 `Idempotency-Key`、須通過限流。據點必須符合 `LIVE_SITE` 且 `accepts_proposals = true`，否則回 409。 |
| `GET /me/proposals` | 提議者查看自己的提議與結果。 |
| `DELETE /proposals/{id}` | 提議者撤回待審提議。 |
| `GET /orgs/{org_id}/sites/{wid}/proposals?status=` | 審核者列表。回應**不含** `user_id` 或任何提議者資訊。 |
| `POST /proposals/{id}/accept` | 接受。須帶 `Idempotency-Key`。 |
| `POST /proposals/{id}/decline` | 婉拒。 |

全部路由放在 `ORGS_ENABLED` 總開關之後（`lib.rs::orgs_gate`）。

## 併發與鎖序

- **接受**：
  1. 先鎖 `wishlists` 列（FOR SHARE），重新檢查據點仍為 `LIVE_SITE`。
  2. 用條件式 UPDATE 改提議狀態：`UPDATE item_proposals SET status = 'accepted' ... WHERE id = $1 AND status = 'pending'`，影響 0 列就回 409。
  3. 新增品項，再新增認領。

  品項是新建的，不會與既有的「`wishlist_items` → `claims`」鎖序衝突。
- **婉拒、撤回、過期**：一律用 `WHERE status = 'pending'` 的條件式 UPDATE，避免與接受互相覆蓋。
- **據點暫停、關閉、組織撤銷驗證**：同一交易內把該據點的 `pending` 提議轉為 `expired`。鎖序接在既有的 `organizations` → `relief_events` → `wishlists` 之後。
- **mutant**（`scripts/mutate.py`）：
  - `proposal-accept-pending-only`：拿掉 `status = 'pending'`，應被重複接受的測試殺掉。
  - `proposal-accept-live-site`：拿掉據點重驗，應被「暫停後接受」的測試殺掉。

## 檢舉與封鎖

提議是陌生人能直接送到組織面前的自由文字，必須讓組織能擋人。難點在組織看不到提議者，所以封鎖以提議為單位操作，系統在背後記錄使用者。

- **封鎖**：審核者在某筆提議上按「封鎖此提議者」。新表 `org_proposal_blocks (org_id, user_id, source_proposal_id, created_by, created_at)`，主鍵 `(org_id, user_id)`；`user_id` 刻意不設外鍵（同 0050）。範圍是**整個組織的所有據點**。封鎖時同一交易內把此人對該組織的 `pending` 提議轉為 `declined`（理由 `other`，不另通知被封鎖原因）。
- **封鎖名單**：`GET /orgs/{org_id}/proposal-blocks` 只回 `{ id, created_at, proposal_name }`（來源提議的品名），不含任何使用者欄位。`DELETE /orgs/{org_id}/proposal-blocks/{id}` 解除。owner / coordinator 可操作。
- **被封鎖者看到什麼**：該組織所有據點的 `accepts_proposals` 對他一律視為 false。分享頁不出現入口，`POST` 回與「據點未開放提議」**相同**的 409。不另發通知。
- **為什麼不沿用 `user_blocks`**：那是使用者對使用者、雙向生效，會刪好友與 selected 名單；組織不是使用者，語意也不同。
- **檢舉**：審核者在提議上按「檢舉」，選理由（沿用現有檢舉理由）。`content_reports` 新增目標 `reported_proposal_id`，0062 的「目標恰好其一」約束改為四選一。檢舉面板預設勾選「同時封鎖此提議者」。
  - staff 在 `/admin/reports` 看得到提議內容與提議者，可沿用 `admin::update_user` 停權。
  - 結案照現有規則發 `report.resolved`，對象是送出檢舉的審核者，payload 只有 `outcome`。
  - 保留期：新目標的 `snapshot` / `detail` 要納入 `ratelimit::REPORT_RETENTION_SQL` 與 `account.rs` 的帳號刪除清理（CLAUDE.md「檢舉保留期」）。
- **稽核**：`proposal.block`、`proposal.unblock`、`proposal.report` 寫 `orgs::audit_user`，diff 只記提議 id 與品名，不記被封鎖者。
- **鎖序**：封鎖先鎖 `organizations`（FOR SHARE）再寫 `org_proposal_blocks` 與 `item_proposals`，與「組織 → 其下資料」的既有鎖序一致。送出提議時在同一交易內檢查封鎖（讀即可，不加鎖）；封鎖與送出同時發生時，晚一步的提議會被封鎖交易一併改為 `declined`，或在封鎖後被擋下，兩種結果都可接受。

## 限流（`ratelimit.rs`）

- 每位使用者對同一據點每天最多 3 筆。
- 每位使用者同時最多 10 筆 `pending`。
- 每個據點同時最多 200 筆 `pending`，超過時入口顯示「暫停受理」。

## 通知（D67，`notify.rs`）

| kind | 對象 | 類別 | Email 層級 | 內容 |
|---|---|---|---|---|
| `proposal.received` | 據點的審核者 | Account（組織） | Daytime | 據點名稱與待審筆數，**不含提議者** |
| `proposal.accepted` | 提議者 | Gifts | Immediate | 品名、數量、認領逾期時間 |
| `proposal.declined` | 提議者 | Gifts | Optional | 品名、理由（含過期） |

- `proposal.received` 不按天合併（處理期限只有 24 小時，按天合併會漏掉當天稍晚的提議）。站內每筆提議即時出現；Email 沿用 worker 既有的合併（尚未寄出的同類通知併成一封）。
- 這不算「有新捐助」通知，不違反 D58。
- 新增 kind 照 CLAUDE.md 清單改齊：`KINDS`、`category`、`email_tier`、`docs/notification-policy.md`、`render` 四語、`inbox_link`。

## 稽核（0059）

- 接受與婉拒寫 `orgs::audit_user`，action 為 `proposal.accept` / `proposal.decline`。
- diff 只記提議 id、品名、數量、新品項 id，**不記提議者**。

## 保留期

- 提議結束（`accepted` 除外）30 天後，清空 `detail` 與 `delivery_note`，與檢舉保留期做法相同。
- `accepted` 的提議內容已轉成品項，同樣 30 天後清空。
- 帳號刪除時，把該使用者的 `pending` 提議轉為 `withdrawn`，並立即清空文字欄位（`account.rs`）。

## 前端

- **分享頁 `/s/{slug}`**：據點開啟提議時，在品項清單下方顯示「我想送清單外的物資」，未登入時顯示「登入後提議」（D59 語感）。
- **提議表單**：用底部滑出面板，與認領面板相同。
- **`/me/claims`**：加一個「我的提議」分頁。
- **組織後台據點頁**：加「物資提議」區塊，提供接受與婉拒的面板。
- 四語文案；新增的創建者路由要加進 `nuxt.config.ts` 的 `routeRules`（`ssr: false`）。

## 測試

- 未登入送出回 401；據點未開啟提議、暫停、關閉時回 409。
- 審核列表與通知內容不含提議者資訊。
- 接受後品項與認領同時存在，認領的 `expires_at` 從接受時間起算。
- 兩個審核者同時接受，只有一個成功（併發測試）。
- 先暫停據點再接受，回 409。
- 24 小時自動過期；據點暫停時待審提議轉為 `expired`。
- 封鎖後：被封鎖者在該組織所有據點看不到入口，`POST` 回應與「未開放提議」完全相同；封鎖名單 API 不含任何使用者欄位；被封鎖者對該組織的待審提議轉為 `declined`。
- 檢舉建立 `content_reports` 且勾選封鎖時同時寫入封鎖；`/orgs/{id}/audit-logs` 不含被封鎖者。
- 限流三條各一個測試。
- 帳號刪除後，提議被撤回並清空文字。

## 待決問題

1. **封鎖要不要有期限**：目前是永久，直到組織手動解除。若試點發現組織常忘記解除，可改為預設 180 天自動失效。
2. **處理期限是否要依災害階段調整**：救災初期可能需要更短（例如 12 小時），重建期可以更長。第一版固定 24 小時，試點後再決定是否開放組織設定。
