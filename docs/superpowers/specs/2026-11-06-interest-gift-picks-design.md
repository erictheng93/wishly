# 依興趣推薦禮物：設計

日期：2026-11-06　狀態：待使用者審閱

## 目的

送禮的人看朋友個人頁、或逛情境頁挑禮物時，依「對方」填的興趣標籤推薦適合的精選商品（`catalog_products`），讓人更容易挑對禮物。延續個人資料擴充（`docs/superpowers/specs/2026-11-06-rich-profile-design.md`）的興趣標籤。

## 目標

1. 精選商品可標「適合的興趣」（同一份 40 個興趣標籤）。
2. 朋友個人頁 `/u/{handle}` 顯示「適合送給 {名字} 的禮物」，最多 6 個。
3. 情境頁 `/inspire/{scene}` 登入後有「幫哪位朋友挑？」選單；選了好友後，符合對方興趣的商品排到最前並加標記，其餘照原順序接在後面（不隱藏）。
4. 配不到時顯示一般推薦（使用者決定）。

## 非目標

- 探索頁依「自己」的興趣排清單（方向 B，使用者未選）。
- 協同過濾、點擊統計、機器學習。
- 推薦清單裡的心願品項（只推精選商品）。

## 反目標

- 推薦結果洩漏看不到的興趣或送禮偏好：觀看者無權看到的興趣 / 不想收到，**不得**影響排序、標記或回應內容。
- 讓已 CDN 快取的 `/public/catalog/products` 讀 cookie。

## 資料

- migration `0072_catalog_interests.sql`：`catalog_products.interests text[] NOT NULL DEFAULT '{}'`；值必須在 `interests::INTERESTS`（應用層驗證，同 recipients / occasions 作法）。
- 後台 `/admin/catalog` 商品表單加「適合的興趣」多選（依 `INTEREST_GROUPS` 分組的 chip，同設定頁）；`POST/PATCH /admin/catalog/products` 收 `interests`，驗證失敗 422 `/interests`。
- 公開 `GET /public/catalog/products` 回應加 `interests`（純商品屬性，不涉個資，可快取）。

## 推薦 API

`GET /users/{handle}/gift-picks?occasion=<birthday|full_month|housewarming|holiday>&limit=<1..50，預設 6>`

- `{handle}` 與 `GET /users/{handle}` 同規則（handle 或登入者可用 user id；停權 / 刪除 404；對方封鎖觀看者 404；觀看者封鎖對方回 `{"basis":"fallback","products":[]}`）。【我的決定】
- 讀觀看者身分（cookie / Bearer），`Cache-Control: no-store`。匿名可呼叫（只會用到 public 欄位）。
- 觀看者是本人時照常計算（自己看自己的推薦）。
- 計算：
  1. 依 `profiles::Rel` / `can_see` 取得**看得到的**興趣（`interests_vis`）與**看得到的**不想收到（`gift_prefs_vis`）；生日沿用 `profiles.rs` 既有規則（`birthday_visibility` 為 `share_level`：public / friends / private）判斷是否看得到。看不到就當作空。`Rel` 的計算（含 `close_friends::is_close`）從 `profiles.rs` 抽成共用函式，gift-picks 與個人頁共用，不複製。
  2. 候選 = 上架商品（`active`），有 `occasion` 時只取該場合。
  3. 排除：商品 `title` 或品牌名稱含任一看得到的「不想收到」字串（不分大小寫、去頭尾空白；字串長度 ≥ 2 才比對，避免單字誤殺）。【我的決定】
  4. 興趣分數 = 商品 `interests` 與看得到的興趣交集數；分數 > 0 的依分數降冪、再依 `sort_order`、`id`。
  5. 有任何分數 > 0 → `basis: "interests"`，回傳這些商品（最多 limit）。
  6. 沒有 → 一般推薦 `basis: "fallback"`：若看得到對方生日且 30 天內（台北日期，`friends::days_until`）且未指定 occasion，候選改為 birthday 場合；否則取 `recipients` 含 `friend` 的商品；依 `sort_order`、`id`，最多 limit。【一般推薦的挑法：我的決定】
- 回應：`{ "basis": "interests" | "fallback", "products": [ { ...與 /public/catalog/products 同形狀..., "matched": ["coffee", ...] } ] }`。`matched` 只含觀看者看得到的興趣；fallback 時為空陣列。
- 一次往返原則：興趣 / 偏好 / 生日讀取併入一個 users 查詢，商品一個查詢（目錄小，排序在應用層）。

## 前端

### 朋友個人頁 `/u/[handle]`

- 在「興趣」區塊後加「適合送給 {名字} 的禮物」區塊：呼叫 gift-picks（limit 6），沿用 `components/guest/CatalogGrid.vue`（含既有「加入我的願望清單」「誰想要這個」動作）。`CatalogGrid` 加一個選填 prop `matched?: Record<string, string[]>`（以商品 ref 為鍵），有值時在卡片上顯示符合的興趣標籤；不傳時現有頁面（`/inspire`、`/inspire/[scene]`、品牌頁）外觀不變。`/u` 是創建者頁，若未載入 `guest.css` 就在該頁引入。【我的決定】
- `basis: interests` 時每張卡顯示符合的興趣標籤（等寬小標籤）；`fallback` 時區塊標題改「也許他會喜歡」之類的一般文案（四語）。
- 本人看自己的頁面不顯示此區塊。【我的決定：避免自我推薦怪異】
- 被封鎖關係不顯示。
- API 失敗：整區不顯示，不影響其他區塊。

### 情境頁 `/inspire/[scene]`

- 登入且有好友時，商品列表上方顯示「幫哪位朋友挑？」選單（`GET /friends`），預設「不指定」= 現行行為。
- 選定好友後呼叫 gift-picks（`occasion` = 該情境的 occasion，若情境沒有 occasion 則不帶；limit 50），在 client 端合併：picks 回傳的商品依回傳順序置頂，`useCatalog` 的分頁結果（含「載入更多」後的頁）以 ref 去重後接在後面，不改 `useCatalog`。【我的決定】置頂的商品卡片加「符合 {名字} 的興趣：咖啡」標記；其餘商品維持原順序接在後面。`fallback` 時不重排，只顯示一行提示「{名字} 還沒有可參考的興趣，以下是一般推薦」。
- 選擇存在網址 query（`?for=<user_id>`），可分享給自己、重新整理不遺失；不存 localStorage。【我的決定】
- 情境頁 SSR 與 CDN 快取不變：選單與重排都在 client 端，登入後才出現。

## 限制

- CLAUDE.md：公開快取端點不可讀 cookie；`slug`/`handle` 比對規則；四語文案；UI 規則（UiIcon、無 eyebrow / 編號）。
- 不新增會讓登入失效的欄位，SESSION_CACHE 不需改。
- 新 migration `0072`。

## 交給 builder

- 商品卡元件的選用與擺放細節、標記樣式（遵守紅白實驗室 token）。
- 測試檔切分（建議後端 `tests/gift_picks.rs`，前端 `tests/giftPicks.test.ts`、e2e 一支）。

## 待確認

無。
