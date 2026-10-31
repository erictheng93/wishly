# 分享頁訪客追蹤與數據分析修正

日期：2026-10-31

## 目的

`/admin/analytics` 的活躍只算登入會員，打開分享連結的未登入訪客完全不計。訪客權杖自 D50 起不再建立，所以「訪客」「新訪客」「送禮者：訪客」三處永遠是 0。本次用真實的瀏覽紀錄取代它們。

## 使用者決定

- 記錄未登入訪客打開分享連結，包含 IP、裝置、地點。
- 完整 IP 不設期限，永久保存。任何清理工作都不可刪除這張表。

## 資料

新表 `share_views`（migration 0051），只增不改：

| 欄位 | 說明 |
|---|---|
| `id` | uuid v7 |
| `wishlist_id` | FK，ON DELETE SET NULL，清單硬刪後紀錄仍保留 |
| `visitor_id` | uuid，可為 NULL，瀏覽器 localStorage 的隨機識別碼 |
| `ip` | inet NOT NULL，取自 `ratelimit::Peer`，不信任 X-Forwarded-For |
| `country`、`city` | text，取自 `CF-IPCountry`、`CF-IPCity`，只在 `TRUSTED_PROXY=cloudflare` 時讀取，否則 NULL |
| `device` | `mobile`、`tablet`、`desktop` |
| `user_agent` | text，截斷至 512 字元 |
| `referrer_host` | text，可為 NULL，只存網域 |
| `created_at` | timestamptz |

索引：`created_at`、`wishlist_id`。

不存使用者 id：只記未登入訪客。不用 ON CONFLICT：每次瀏覽一列，不重複訪客靠 `COALESCE(visitor_id::text, ip::text)` 計算。

## 記錄方式

`GET /public/wishlists/{slug}` 會被 CDN 快取且不可讀 cookie，所以不在那裡記錄。分享頁掛載後，未登入者送一次 `POST /public/wishlists/{slug}/views`：

- body：`{ visitor_id?: uuid, referrer_host?: string }`，格式不符的欄位當作 NULL。
- 一律回 204，不洩漏清單是否存在；找不到或已刪除就不寫入。
- 爬蟲不寫入：UA 不分大小寫含 `bot`、`crawl`、`spider`、`facebookexternalhit`、`preview`、`slurp`、`whatsapp`，或 UA 為空。
- 依 IP 限流（`ratelimit::allow`，60 次／分鐘），超過仍回 204 但不寫入。
- 排除範例清單（slug 以 `demo` 開頭）。
- `read_only` 模式維持全域行為。

前端 `utils/shareView.ts`：`credentials: 'omit'`、`keepalive: true`，所有錯誤吞掉，絕不影響分享頁。localStorage 不可用時不帶 `visitor_id`。原生殼與已登入者不送。

## 數據分析

`GET /admin/analytics` 調整：

- `totals.guests` 移除，改 `totals.visitors`（歷來不重複訪客）。
- `daily[].new_guests` 移除，改 `visitors`、`views`。
- `claims.by_actor` 移除。
- 新增 `visits`：`{ views, visitors }`（區間內）。
- `distribution` 新增 `view_device`、`view_country`、`view_referrer`，沿用少於 5 抑制。

頁面顯示不含 IP，也沒有任何 API 回傳 IP；要查只能直接查資料庫。

## 順手修正

- `L.wishlistVisibility` 改為 `public`、`friends`、`selected`、`private`，語系補齊。
- `console.css` 的 `.c-badge`、`.an-dates` 由 10.5px 改 11px。

## 文件

隱私權政策新增「瀏覽紀錄」段落，如實寫明蒐集項目、目的與長期保存，所有語系的 `legal.json` 都要補。`docs/04` 補契約，`CLAUDE.md` 補一行。

## 非目標

後台查 IP 的介面、單一清單的瀏覽報表、停用追蹤的開關。
