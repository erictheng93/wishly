# 通知政策

站內通知是主要管道，Email 是補充。這份文件是通知行為的唯一來源：新增或調整通知時，改 `apps/api/src/notify.rs` 的 `category`、`email_tier`，並同步更新這裡。

## 站內通知

- 所有會員通知都會進站內，只有兩種情況例外：驚喜鎖定中的受贈者端通知，以及使用者關閉了該類站內通知。
- 站內通知不受安靜時段、Email 延後或 Email 開關影響，寫入後立即提示前端，不需重新整理。
- 帳務、安全與期限類不可關閉。送禮進度、社交與生日可分別關閉站內或 Email（`inbox_gifts`、`email_gifts`、`inbox_social`、`email_social`，缺鍵視為開啟）。

## Email 判斷標準

一封信至少要符合下列一項才值得寄：

1. 錯過會有實際損失：錢、期限、實體物資或法規責任。
2. 需要站外留存證明：安全事件或金額變動。
3. 收件人沒有別的管道：進不了帳號，或沒有帳號。
4. 值得把人帶回來，而且頻率低。

## Email 四級

### 第一級：立刻寄，不可關閉（`Immediate`）

| 類別 | kind |
|---|---|
| 驗證碼 | 登入、註冊、重設密碼（直接寄送，不進站內） |
| 安全 | `auth.password_changed`、`auth.new_device`、`auth.mfa_enabled`、`auth.mfa_reset`、`auth.recovery_regenerated`、`auth.identity_linked`、`auth.identity_unlinked` |
| 帳號處分 | `account.suspended`、`account.restored` |
| 金流 | `topup.credited`、`cash_refund.updated`、`wallet.frozen`、`wallet.unfrozen`、`wallet.chargeback`、`order.failed`、`order.partial_refund`、`funding.expired`、`funding.released`、`item.removed` |
| 組織與治理 | `org.invite`、`org.review`、`org.permit_reviewed`、`org.permit_rejected`、`org.permit_revoked`、`org.permit_expiring`、`org.permit_expired`、`org.event_hidden`、`wishlist.moderated` |
| 緊急期限 | `site.paused`、`wishlist.closing` |
| 營運 | `staff.digest` |

### 第二級：不可關閉，09:00 才寄（`Daytime`）

`claim.purchase_reminder`、`claim.list_closed`、`claim.cancelled_by_owner`、`claim.item_removed`。

例外：`claim.purchase_reminder` 的期限早於下一個 09:00 時立刻寄。

### 第三級：預設開、可關閉，09:00 才寄（`Optional`）

| 對象 | kind |
|---|---|
| 捐助者 | `claim.expired`、`claim.item_changed`、`crowdfund.funded`、`order.shipped`、`order.delivered` |
| 擁有者 | `owner.funded`、`owner.shipped`、`owner.delivered`、`owner.order_failed`、`event.summary` |
| 社交 | `friend.request`、`birthday.friend`、`birthday.nudge` |

### 第四級：會員不寄，只留站內（`InAppOnly`）

`claim.confirmation`、`friend.accepted`、`item.reopened`、`claim.cancelled_by_donor`、`topup.expired`。

舊訪客沒有站內通知，這些 kind 若寄給舊訪客仍照常寄信。

## Email 寄送規則

- **安靜時段：** 台北 21:00–09:00，第二、三級延後到 09:00，不算寄送嘗試。
- **品項修改：** 每次修改都立刻出現一則站內通知。Email 從修改起延後 10 分鐘，同一人同一品項的修改併成一封，內容以寄出當下為準；寄出後至少隔 1 小時才寄下一封。窗口內取消支持就不寄。
- **同類合併：** `claim.purchase_reminder`、`claim.expired` 等同收件人的同類 Email 併成一封，站內各自保留。
- **重試：** 最多 6 次，退避 1m / 5m / 30m / 2h / 6h。營運可重新排入近 7 天的失敗通知。
- **退訂連結：** 只有第三級的會員信，以及寄給舊訪客的信才附。
- **不寄的情況：** 帳號沒有 Email 時只有站內通知。

寄信與站內寫入分開：一批通知先全部寫入站內，再以最多 5 個併發寄信，SMTP 卡住不影響站內。

## 推播

Web Push 是 APP 外的提醒，開關在每台裝置上，需使用者主動開啟。

- **時機與合併：** 跟 Email 相同。安靜時段延到 09:00（第一級除外），品項修改 10 分鐘合併、同人同品項至少隔 1 小時。
- **第四級不推：** 使用者剛做完或不需要處理的事，留在站內即可。
- **與 Email 無關：** 關閉 Email 或沒有 Email 地址，仍然會推播。關閉該類站內通知才會停止。
- **內容：** 只有標題與站內路徑，不放內文，避免在鎖定畫面外洩。
- **失敗：** 只記 log，不重試，也不影響通知狀態。

原生殼（Tauri）目前不支援系統推播，只有 App 開著時的站內通知。

## 內容底線

通知不得帶捐助者身分，也不新增「有新捐助」通知（D58）。驚喜鎖定中，受贈者端通知一律不寄、不進站內。

## 即時連線

| 項目 | 公開頁 SSE | 站內通知串流 |
|---|---|---|
| 驗證 | 匿名 | 登入 |
| 單位上限 | 每 IP 16 條 | 每位使用者 6 條 |
| 全站上限 | 2000 條 | 4000 條 |

兩邊名額各自獨立，互不排擠。站內串流按使用者計算，所以共用對外 IP 的行動網路（CGNAT）使用者不會互相影響。

- **背景分頁：** 隱藏 1 分鐘後斷線，回到前景重連並補抓一次。
- **避免同時湧入：** 重連延遲隨機放大最多 50%，連線存活 25–35 分鐘依使用者分散。
- **名額已滿（429 / 503）：** 前端立刻改為每 15 秒輪詢，連線恢復後停止。
- **斷線偵測：** 45 秒沒收到任何資料（伺服器每 15 秒心跳）就重連，涵蓋休眠或切換網路。
- **請求合併：** 一連串訊號只會觸發一次進行中的抓取，加上結束後一次補抓。

### 容量實測（2026-10-31）

本機單一 API 程序（debug build），未經 TLS 與 Cloudflare Tunnel：

| 指標 | 結果 |
|---|---|
| 2000 條連線 | 全部成功，約 1.1 秒建立 |
| 超過上限 | 4300 條中 4000 條成功，其餘 300 條回 503 |
| 記憶體 | 每條約 52 KB，4000 條時常駐約 150 MB |
| 訊號到達延遲 | p50 約 7 毫秒，p99 約 49 毫秒 |

### 上線後觀察

沒有 metrics 系統，API 每小時寫一筆 log：沒有被拒時是 info `sse connections`，有被拒時是 warn `sse connections rejected`，欄位為 `public`、`public_rejected`、`inbox`、`inbox_rejected`。

`inbox_rejected` 持續大於 0 時，先確認 VPS 記憶體與檔案描述元上限，再調高 `apps/api/src/inbox.rs` 的 `MAX_STREAMS_TOTAL`。
