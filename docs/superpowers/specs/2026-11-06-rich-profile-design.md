# 會員個人資料擴充：設計

日期：2026-11-06　狀態：待使用者審閱

## 目的與理由

Wishly 是送禮產品。個人資料的價值在於讓朋友**挑對禮物、記得日子、送到對的地方**，以及讓用戶有一個可分享的對外名片（Linktree 式連結頁）。本次一次補齊七塊：送禮偏好、興趣標籤、重要日子、地址（收件 / 居住）與年齡、外部連結與極簡連結頁、個人頁封面、帳號層級摯友名單。

## 目標

1. **帳號摯友名單**：每位用戶一份，只能從好友中挑；清單的 `selected` 可見性改為「帳號摯友名單上的人」，取代每份清單各自勾選的 `wishlist_allowed_users`。
2. **送禮偏好**：「偏愛」與「不要收到」兩組短句清單。
3. **興趣標籤**：從程式內建標籤庫選，不可自由輸入。
4. **重要日子**：用戶自行增刪；7 天前站內通知看得到該日子的人；個人頁顯示。
5. **預設收件地址**：建立清單時自動帶入清單收件資訊；與居住地址分開；永不顯示於個人頁。
6. **居住地址、年齡（出生年）**：預設不顯示於個人頁；用戶可選「好友 / 摯友 / 只有自己」。
7. **外部連結 + 極簡連結頁**：獨立、像 Linktree 的分享頁。
8. **個人頁封面**：沿用清單封面的 `COVER_PRESETS` 與上傳流程。

## 非目標

- 用興趣標籤做探索頁 / 情境頁推薦（本次只存與顯示；推薦另開任務）。【我的決定】
- 個人資料完成度百分比 / 進度條（與 `creator/AccountNudge.vue` 重複，不做）。
- 尺寸、過敏等結構化偏好欄位（用戶可寫成一行偏好短句）。【我的決定】
- 外部連結點擊統計、連結預覽抓取（伺服器不抓用戶網址，避免 SSRF）。

## 反目標（做出來但算失敗）

- 任何受限欄位（住址、年齡、偏好、日子等）出現在沒權限者的 API 回應、CDN 快取、OG 預覽或日誌裡。
- 地址、電話出現在稽核 diff、檢舉快照、匯出以外的任何地方。
- 摯友搬移後，原本看得到某份 selected 清單的人看不到了（合併只能擴大、不能縮小）。

## 可見性模型

新 enum `profile_vis`：`public` / `friends` / `close` / `private`。

| 欄位 | 可選 | 預設 |
|---|---|---|
| 送禮偏好（偏愛 + 不要，共用一個設定） | public / friends / close / private | friends【我的決定】 |
| 興趣標籤 | public / friends / close / private | public【我的決定】 |
| 重要日子（每筆各自設定） | public / friends / close / private | friends【我的決定】 |
| 居住地址 | friends / close / private | **private**（使用者決定） |
| 年齡（出生年） | friends / close / private | **private**（使用者決定） |
| 預設收件地址 | 不可設定，永不顯示 | — |
| 外部連結 | 每個連結「顯示 / 隱藏」；顯示即公開 | 顯示 |
| 封面 | 永遠公開（同頭像） | — |

- 判斷：`public` 任何人；`friends` 本人或好友；`close` 本人或在本人摯友名單上；`private` 只有本人。被封鎖關係一律不回（沿用 `profiles.rs` 現行封鎖處理）。
- 設定頁 UI：住址 / 年齡用一個選單「在個人頁顯示：不顯示（只有我）/ 好友 / 摯友」，不另做勾選框。
- 既有生日可見性（`share_level`：public / friends / private）本次**不改**。【我的決定：避免動到生日通知邏輯；之後要加 close 再說】

過濾一律在後端 `GET /users/{handle}`（會讀 cookie、不經 CDN）完成，看不到的欄位不出現在回應中（不是 null 加旗標）。

## 1. 帳號摯友名單

- 表 `close_friends (owner_id, friend_id, created_at, PK(owner_id, friend_id))`，兩欄皆 FK users ON DELETE CASCADE；索引 `(friend_id)`。
- 加入時必須是好友（交易內檢查 `friendships`）。上限 200 人。【我的決定】
- 解除好友（`friends.rs` 391 附近）與封鎖（`blocks.rs` 64 附近）原本刪 `wishlist_allowed_users` 的地方，改刪雙向 `close_friends`。
- `friends.rs` 515「接受 selected 清單邀請自動加入名單」改為 INSERT `close_friends (清單擁有者, 接受者)`。
- 所有讀 `wishlist_allowed_users` 的查詢改為 `EXISTS (SELECT 1 FROM close_friends c WHERE c.owner_id = w.owner_id AND c.friend_id = $viewer)`：`access.rs`、`birthday.rs`、`friends.rs` 579、`inspire.rs`、`notify.rs` 719、`profiles.rs` 158/197。
- `wishlists.rs` 2177 起的 selected 名單 API（GET/PUT）：GET 改回傳帳號摯友名單（相容前端）、PUT 移除；新 API：
  - `GET /me/close-friends`、`PUT /me/close-friends/{user_id}`、`DELETE /me/close-friends/{user_id}`。
  - 被列為摯友不通知（舊 PUT 本來也不發通知）。
  - 舊 `GET/PUT /wishlists/{id}/allowed-users` 直接刪除，前後端同一個 PR、同一次發版；後端先上線時舊前端儲存名單會短暫失敗，可接受。【我的決定】
- 前端：`VisibilityPicker.vue` 的 selected 勾選面板改為顯示「N 位摯友」＋連到設定頁摯友卡；設定頁新增「摯友名單」卡（從好友中加入 / 移除）；好友列表每列加「設為摯友」切換。i18n 的 selected 描述改為「只有你的摯友名單能開」。
- **遷移**：先把名單為空（沒有任何仍為好友的列）的個人 selected 清單改為 `private`（使用者決定：維持原本誰都看不到），再`INSERT INTO close_friends SELECT DISTINCT w.owner_id, a.user_id FROM wishlist_allowed_users a JOIN wishlists w ... WHERE w.org_id IS NULL AND 仍是好友 ON CONFLICT DO NOTHING`，然後 `DROP TABLE wishlist_allowed_users`。使用者已接受此合併會擴大部分清單的可見範圍。
- 組織據點不會有 selected（`org_sites.rs` 只建 relief、`no_relief` 擋名單 API）；遷移前先斷言 `SELECT count(*) FROM wishlists WHERE org_id IS NOT NULL AND visibility='selected'` 為 0，不為 0 就把那些列改為 private 並 RAISE NOTICE。【我的決定】
- 鎖：`close_friends` 寫入與好友解除 / 封鎖同屬一對使用者，沿用 `friends::lock_pair`（此鎖一律最先取）。
- 稽核：舊 PUT 寫的 `wishlist.allowed_users.update` 改為 `close_friends.add` / `close_friends.remove`（`security::audit` 同型 helper，diff 只記對方 user id），維持可追溯。【我的決定】

## 2. 送禮偏好

- `users.gift_likes text[]`、`users.gift_dislikes text[]`，各最多 20 條、每條 1–40 字（去頭尾空白、去重、不可空）；`users.gift_prefs_vis profile_vis DEFAULT 'friends'`。
- PATCH `/me` 收整個陣列覆寫（順序即顯示順序）。
- 設定頁「送禮偏好與興趣」卡：兩個清單各有輸入框＋新增、每條可刪除與上下移動（不做拖曳函式庫）。
- 個人頁：兩欄（手機上下）「偏愛」「不要收到」，等寬小標籤 chip 樣式。
- 會被檢舉快照納入（使用者檢舉的 `snapshot`），所以要確認 `REPORT_RETENTION_SQL` 只清 snapshot 即涵蓋，不另加欄位處理。

## 3. 興趣標籤

- 標籤庫：前後端各一份 id 常數（前端 `utils/interests.ts`，後端 `interests.rs` 的 `const INTERESTS: &[&str]`），id 為小寫 kebab；約 40 個、6 類（生活、美食、運動、藝文、科技、親子寵物）；四語翻譯在 `i18n/locales/*/interests.json`。builder 擬定清單後列在 PR 給使用者看。
- `users.interests text[]` 最多 10 個、須全在 `INTERESTS`；`users.interests_vis profile_vis DEFAULT 'public'`。
- 設定頁：依類別分組的可切換 chip，超過 10 個時其餘停用。
- 個人頁：chip 列。
- 為之後推薦預留：GIN 索引 `users (interests)` 本次**不建**（沒有查詢用到）。

## 4. 重要日子

- 表 `user_dates (id uuid PK, user_id FK users ON DELETE CASCADE, title text 1–30 字, month_day text 'MM-DD'（同 birthday 的 CHECK）, year smallint NULL, recurring boolean NOT NULL DEFAULT true, visibility profile_vis NOT NULL DEFAULT 'friends', created_at)`；每人最多 20 筆（API 檢查）。
- `recurring = false` 必須有 `year`；非重複且日期已過的不再通知、個人頁不顯示（設定頁仍列出，標「已過」）。
- API：`GET /me/dates`、`POST /me/dates`、`PATCH /me/dates/{id}`、`DELETE /me/dates/{id}`。
- 通知：新 kind `profile.date_soon`，InAppOnly、Social 類別，7 天前（台北時區，同生日 tick 的日期算法）發給「看得到這筆日子的人」：public / friends → 當下所有好友；close → 當下摯友；private 不發；public 不發給非好友。
  - 冪等：表 `user_date_sent (date_id FK user_dates ON DELETE CASCADE, year smallint, PK(date_id, year))`，比照 `wishlist_birthday_sent`：同交易 INSERT ... ON CONFLICT DO NOTHING 成功才 enqueue 給收件人集合。收件人在發送當下計算，之後新加的好友不補發。
  - 補發窗口：日子在今天起 0–7 天內且該年未發就發（停機後晚到照發，日子已過不發）。
  - 收件人排除：刪除 / 停權、任一方向封鎖、`inbox_social = false`（InAppOnly 只看站內偏好）。
  - 改日期或可見性：刪除該筆當年的 `user_date_sent`，讓它可以重新判斷。
  - `user_dates` 不再需要 `notified_year` 欄位。
- 新 kind 照 CLAUDE.md 清單補齊：`KINDS`、`category`、`email_tier`、`docs/notification-policy.md`、`render` 四語、`inbox_link`（連到 `/u/{handle}`）。
- 個人頁：生日那行旁顯示「接下來」最近一筆可見日子（名稱 + 日期 + 倒數天數）；另有一個完整列表區塊。

## 5. 地址與年齡

- 預設收件地址：表 `user_addresses (user_id, kind 'shipping'|'home', recipient_name_enc, phone_enc, address_enc, updated_at, PK(user_id, kind))`，以 `sealed::seal` 加密，與 `shipping_addresses` 同格式。【我的決定：一張表兩種 kind，而不是兩張表】
  - `home` 只存 `address_enc`（姓名、電話欄位可 NULL）；`users.home_address_vis profile_vis DEFAULT 'private'`，CHECK 不可為 public。
  - API：`GET /me/addresses`（回遮罩值，同 `ShippingForm` 現行）、`PUT /me/addresses/{kind}`、`DELETE /me/addresses/{kind}`。
  - 清單收件資訊帶入：`ShippingForm.vue` 在清單尚無收件資訊且帳號有 shipping 地址時，顯示「使用預設收件地址」按鈕，後端 `POST /wishlists/{id}/shipping-address/from-default` 在伺服器端複製密文（明文不回前端）。【我的決定：按鈕而非自動寫入，避免用戶不知情就把地址綁到清單】
  - 個人頁：home 依可見性顯示**完整明文**（使用者決定）；shipping 永不顯示。
- 年齡：`users.birth_year smallint CHECK (1900 ≤ year ≤ 今年)`，`users.age_vis profile_vis DEFAULT 'private'`，CHECK 不可為 public。設定頁放在生日欄位旁；個人頁顯示「N 歲」（依今天與生日 MM-DD 計算，無 MM-DD 時以年份差）。
- 稽核：地址只記 `{"field":"home_address","changed":true}`，不記內容。
- 帳號刪除（`account.rs`）：刪 `user_addresses`、`user_dates`，清空上述 users 欄位。
- 資料匯出（`/me/export`）：包含明文地址、日子、偏好、標籤、連結。【我的決定：匯出是本人資料】

## 6. 外部連結與極簡連結頁

- 表 `user_links (id uuid PK, user_id FK CASCADE, title text 1–40, url text, position smallint, hidden boolean DEFAULT false, created_at)`，每人最多 10 個。
- URL 驗證（後端，信任邊界）：`https` 才收、長度 ≤ 500、可被 `url::Url` 解析、無 userinfo；不抓網址內容。
- 圖示：前端依 host 對應（instagram.com、threads.net、youtube.com / youtu.be、line.me、facebook.com、tiktok.com、x.com、github.com），其餘用通用連結圖示；全部走 `UiIcon`（缺的圖示 builder 補 1.5px 線條 SVG）。
- API：`GET /me/links`、`PUT /me/links`（整組覆寫：順序、隱藏、增刪一次送）。
- 連結頁：前端 `/l/[handle]`，SSR（給 LINE / FB 預覽），內容只有封面（有的話）、頭像、名稱、bio、連結按鈕，最後一顆固定按鈕「看我的願望清單」→ `/u/{handle}`。沒有 handle 的人沒有連結頁（同個人頁規則）。
  - 資料來源：新公開端點 `GET /public/links/{handle}`，**不讀 cookie**、只回公開內容（名稱、頭像、封面、bio、非隱藏連結），`Cache-Control: public, s-maxage=60`，不帶 ETag，不做 CDN purge（改動最多 60 秒後生效）。【我的決定】
  - 封鎖關係不影響（匿名可看的公開頁本來就無法區分）；停權 / 刪除帳號回 404。
  - `robots: noindex`、不進 sitemap、不加入 `LOCALIZABLE`（同 `/u` 現況）。【我的決定】
  - 連結一律 `target="_blank" rel="nofollow ugc noopener noreferrer"`。
- 設定頁「個人頁外觀」卡：連結列表（新增、刪除、上下移動、隱藏切換）＋「複製連結頁網址」與「開啟」。個人頁也顯示連結區塊與「分享連結頁」按鈕。

## 7. 個人頁封面

- `users.cover_preset text`（同 0032 的 CHECK 值）、`users.cover_image_key text`，互斥。
- 上傳：沿用 `purpose="cover"`，但完成時要寫 users 而非 wishlists —— 新增 purpose `profile_cover`（目錄可共用 `covers/`），`uploads.rs` 的「圖片是否仍被引用」檢查（541 行附近）加上 `users.cover_image_key`，否則孤兒清理會刪掉。0034 的使用者媒體清單也要納入。
- 前端：`CoverPicker` 加 `purpose` prop（預設 `cover`），設定頁「個人頁外觀」卡使用。個人頁主視覺最上方顯示，無封面時不留空白。

## 設定頁結構

`settings.vue` 已 380 行，新卡片各自拆成元件放 `components/creator/profile/`：`AppearanceCard`（封面、bio、連結）、`GiftPrefsCard`（偏好、標籤）、`DatesCard`、`PrivateInfoCard`（年齡、居住地址、預設收件地址）、`CloseFriendsCard`。現有 `#social` 卡保留 handle、生日、認捐可見性；bio 移到 AppearanceCard。每張卡各自儲存。

## 個人頁結構（`/u/[handle]`）

封面 → 主視覺（頭像、名稱、@handle・生日・年齡、bio、最近重要日子、共同好友、動作鈕）→ 連結 → 送禮偏好 → 興趣 → 居住地址 → 重要日子 → 清單 → 捐助。沒有內容或沒權限的區塊整塊不出現。樣式沿用紅白實驗室 token，不加 eyebrow / 編號。

## 限制

- CLAUDE.md 的鎖序、通知新增 kind 清單、檢舉保留期、稽核不記個資、`slug`/`handle` 比對規則都適用。
- 改動 `selected` 存取查詢屬存取控制變更：`scripts/mutate.py` 要補 mutant（例如拿掉 close_friends 的 owner 條件），並跑通相關切片。
- 新 migration 從 `0067` 開始。
- `SESSION_CACHE`：本次不新增會讓登入失效的欄位，不需改 0065。

## 交給 builder 決定

- 標籤庫的具體 40 個 id 與四語翻譯（PR 中列出給使用者看）。
- 元件內部切分、排序 UI 的細節（上下移動按鈕即可）。
- 測試切片的檔名與分法（建議 `tests/close_friends.rs`、`tests/profile_fields.rs`、`tests/user_dates.rs`、`tests/user_links.rs`）。

## 待確認

無。
