# APP 內通知改為主要管道（A 階段）

使用者要的是 APP 內通知，且必須即時：不重新整理頁面，通知就要出現。Email 退為補充管道。
Web Push / 手機推播是下一階段（B），這裡不做，但通知內容要能直接沿用。

## 固定決定

- 站內通知不受安靜時段影響；安靜時段只延後 email。
- `claim.confirmation`、`claim.item_changed` 站內立即出現；只有 email 從建立起延後 10 分鐘，讓連續操作合併，已取消的不寄。
- 即時訊號只是提示，不帶內容：`event: inbox`、`data: {}`。內容一律由已驗證的 REST 取得，連線存活期間登出也不會外洩。
- Email 偏好預設值不變（未設視為開）。
- 這個分支還沒上線，直接改 `0048`，不新增 migration。

## 後端（apps/api、db/migrations）

### 資料

`0048` 加 `inbox_body text`、`inbox_category text`。

### 分類

下列 kind 改為 `Category::Account`（會造成損失，email 不可關，不受安靜時段延後）：
`claim.purchase_reminder`、`claim.expired`、`claim.cancelled_by_owner`、`claim.list_closed`、`claim.item_removed`、`wishlist.closing`、`site.paused`。

### 偏好

新鍵 `inbox_gifts`、`inbox_social`（布林，未設視為開），`auth.rs` 一併放行。Account 類不看偏好。

`birthday.rs` 目前在排入時就依 `email_social` 過濾，關掉 email 會連站內都收不到。改成：`email_social` 與 `inbox_social` 都關才跳過。

### worker 順序（notify.rs）

領取 → 收件人 → 寄前重驗 → 驚喜把關 → 合併 → render → 寫站內 + 發訊號 → email 偏好靜音 → 安靜時段延後 email → 寄送。

- 領取時多回傳 `inbox_at IS NOT NULL`（下稱 has_inbox）。has_inbox 為真時不再更新站內欄位，也不再發訊號：標題凍結在第一次寫入。
- 站內寫入條件：`uid` 存在、`!has_inbox`，且 Account 類或對應的 `inbox_*` 未關。
- 合併查詢加條件 `(n.inbox_at IS NOT NULL) = has_inbox`。第一輪只併還沒進站內的列，避免列被吞掉或重複；隔天 09:00 第二輪只併已進站內的列，email 仍然合成一封。
- 寄信與前面的 DB 工作分開：`tick_at` 先對整批做完 DB 階段（站內立即寫入），再以併發上限 5 寄信（`for_each_concurrent` 同類做法），SMTP 卡住不能擋住站內通知。`tick_at` 回傳時兩階段都已完成，測試行為不變。
- `spawn_worker` 間隔 5 秒改 1 秒。先確認 pending 掃描有索引；沒有就在 `0048` 補部分索引。

### 內文

`pub fn inbox_body(email_body: &str) -> String`：去掉含 `://` 的行與空行，其餘以 `\n` 連接。單元測試：`KINDS` × 四語，結果都非空且不含 `://`（payload 用既有測試的做法）。

### 即時串流

沿用 `dashboard.rs`：`dashboard::notify(ex, user_id)` 共用 `wishlist_events` 通道（UUID 不會與清單 id 相撞，清單訂閱者只比對自己的 id，加一行註解說明），新增 `pub(crate)` 的訂閱函式回傳 `broadcast::Receiver`。不要另起 PgListener。

`GET /me/notifications/stream`（`CurrentUser` 驗證，所以 cookie 與 Bearer 都能用）：

- 沿用 `SseSlot` 名額、首個 `retry` 事件、心跳與 no-cache 標頭。
- 只在收到自己 user id（或 broadcast `Lagged`）時送 `event: inbox`、`data: {}`。
- 連線最長 30 分鐘後結束，客戶端重連時重新驗證 session。
- `read_one`、`read_all` 成功後也發訊號，讓其他分頁同步未讀數。

### 列表 API

`GET /me/notifications` 每筆多回 `body`、`category`（`account|gifts|social`）；支援 `?category=`，其他值回 422。帳號匯出與刪除要涵蓋新欄位。

### 測試

更新 `notify_foundation.rs` 並新增：

- 安靜時段：Gifts 列站內已寫入、email 延到 09:00、第二輪不重複發訊號也不改標題。
- 夜間兩列第一輪各自進站內，隔天 09:00 email 合併成一封，兩筆站內都保留。
- `inbox_social=false`：不進站內但照寄 email；Account 類不受影響。
- 關閉 `email_social` 仍收到 `birthday.friend` 站內信。
- SMTP 失敗的列不影響同批其他列寫入站內。
- 串流測試：`BUS` 是綁第一個 pool 的全域單例，測試放獨立檔案且只用一個 `#[sqlx::test]`。驗證本人收到、別人收不到、未登入 401。

## 前端（apps/web）

- `utils/inboxStream.ts`：純函式、不依賴 Vue。用 `fetch` 讀串流（`EventSource` 不能帶 Bearer，網頁與原生共用同一條路），解析 `event: inbox`。斷線退避 2/5/15/30 秒；401/403 停止；連續失敗 3 次起每 15 秒呼叫一次 `onEvent` 當輪詢，連上後停止。`stop()` 用 `AbortController`。每次連上都觸發一次 `onEvent`，補上斷線期間的通知。vitest 要覆蓋解析（含事件被切在兩個 chunk）、重連、401 停止、輪詢。
- `useInbox`：`unread`、`latest`（最新 5 筆）、`version`（每次更新遞增）、`toast`。`refresh()` 打 `/me/notifications?limit=5`，`noRedirect`，失敗保留前值。最新一筆 id 改變且未讀增加時才設 `toast`，首次載入不跳。
- `plugins/inbox.client.ts`：全站唯一一條串流，依 `user` 開關（登出即 stop），`visibilitychange` 回前景時 refresh。帶 `nativeHeaders()` 與 `credentials: 'include'`。
- `Bell`：按下展開面板，顯示最新 5 筆（標題 + 內文兩行截斷）、全部已讀、查看全部。Esc 與點外面關閉，鍵盤可操作，手機寬度不溢出。`GuestTop` 也要有（僅登入時）。
- 全站 toast（掛 `app.vue`）：`aria-live="polite"`，6 秒消失，可點開連結；在 `/me/notifications` 不顯示。
- `/me/notifications`：顯示內文；分類分頁（全部 / 帳務安全 / 送禮 / 好友生日，帶 `?category=`）；`version` 變動時自動重抓第一頁（尚未載入更多時）。
- 設定頁：每類一列、站內與 Email 兩個開關；帳務列兩個都鎖定開啟。文案改為「帳務、安全與期限」。
- 四語 i18n；沿用 `c-` 主題樣式；不輪詢以外的機制不新增。
- e2e `inbox-realtime.spec.ts`：登入後用 psql helper 插入一筆 pending 的 Gifts 類通知，不 reload 即看到徽章與 toast；開第二分頁全部已讀，第一分頁徽章歸零。

## 文件

`docs/04` 契約、PRD D67、`CLAUDE.md` 通知條目（主代理負責）。
