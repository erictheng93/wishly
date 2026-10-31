# 通知系統全面補強 實作計畫

目標：補齊 P0 / P1 通知、修現有機制缺陷、加站內通知中心與三組偏好。LINE Messaging API 不在本次範圍。

## 全域限制

- worktree 分工：Task 1 先完成，Task 2–5 各自 worktree 平行；只改自己任務列出的檔案。
- 新 migration 編號固定 `0048`：`0047` 已被另一條進行中的分支（金流後台）占用。
- **不新增或更動 `FOR UPDATE` / `FOR SHARE` / `ON CONFLICT` 與鎖序**。通知一律是同交易內的純 INSERT，放在既有業務 SQL 之後；不要改既有 CTE（`expire_due`、`grace_on_close` 等）的鎖定行為。若真的非改不可，停下來回報。
- 通知與業務寫入同交易：rollback 就不寄。
- D58 不變：不新增「有新捐助」即時通知。驚喜鎖定中，受贈者一律收不到任何通知（由 worker 集中把關，見下）。
- 通知內容不得帶捐助者身分（名字、email）給受贈者；`event.summary` 只帶人數。
- 文件與註解繁體中文，比照周圍密度。commit 訊息不加任何 Claude 署名或 Co-Authored-By。
- 測試：新測試放 `apps/api/tests/notify_<task>.rs`，用 `#[sqlx::test(migrations = "../../db/migrations")]`。**只跑自己的測試檔與直接相關的既有測試**：多個 worktree 同跑同一測試會撞測試庫名（23505）。全套由主代理最後跑。
- 指令：`export DATABASE_URL=postgres://wishly:wishly@localhost:5432/wishly APP_ENV=dev`；`cargo test --manifest-path apps/api/Cargo.toml --test <file>`。Docker 的 db / mailpit 已在跑，不要動它們。

## 通知類別（kind registry）

`notify.rs` 集中定義 `category(kind)`：

| 類別 | 可關閉 | 安靜時段 | kinds |
|---|---|---|---|
| Account | 否 | 不延後 | `auth.*`、`org.*`、`staff.digest`、`account.*`、`wallet.*`、`topup.*`、`cash_refund.updated`、`wishlist.moderated`、`order.failed`、`order.partial_refund`、`funding.expired`、`funding.released`、`item.removed` |
| Gifts | `email_gifts` | 延後（`wishlist.closing` 除外） | 其餘全部 |
| Social | `email_social` | 延後 | `friend.*`、`birthday.*` |

## 新增 kinds 與 payload

所有含清單 / 品項的 payload 都帶 `wishlist_id`、`item_id`（沿用 worker 既有的 `title`、`item_title` 查詢）。

| kind | 收件人 | payload（額外欄位） | 內容重點 |
|---|---|---|---|
| `claim.confirmation`（沿用，改寫文案） | 捐助者（會員） | `claim_id`, `expires_at` | 已記下由你送「品項」，請在期限前標記已購買，連結我的送禮 |
| `claim.cancelled_by_owner` | 捐助者 | | 清單擁有者取消了你的認領；若已購買請自行處理 |
| `claim.cancelled_by_donor` | 受贈者 | | 一份已購買的禮物被取消了，品項重新開放；不提是誰 |
| `funding.released` | 捐贈者 | | 7 天選擇期結束，點數已自動退回錢包 |
| `order.partial_refund` | 捐贈者 | | 實際花費低於目標，差額已依比例退回 |
| `owner.funded` / `owner.shipped` / `owner.delivered` / `owner.order_failed` | 受贈者 | | 你的「品項」達標／已出貨／已送達／代購未成功 |
| `event.summary` | 受贈者 | `givers`（人數） | 有 N 位朋友為《清單》送上心意（取代 `event.reminder`） |
| `site.paused` | 據點進行中的捐助者 | | 據點暫停，先別寄出物資，到我的送禮確認 |
| `friend.accepted` | 申請者 / 邀請人 | `from_user`（worker 查 `from_name`） | {name} 接受了你的好友邀請，連到 `/friends` |
| `topup.credited` | 儲值者 | `amount` | 儲值 NT$N 已入帳 |
| `topup.expired` | 儲值者 | `amount` | 繳費期限已過，訂單已失效 |
| `cash_refund.updated` | 申請者 | `amount`, `status`(completed/rejected), `note` | 現金退款結果 |
| `wallet.frozen` / `wallet.unfrozen` | 持有者 | | 錢包已凍結 / 解凍，附客服指引 `/terms` |
| `wallet.chargeback` | 持有者 | `amount`, `debt` | 銀行爭議扣回 N 點；欠款 M 點 |
| `wishlist.moderated` | 擁有者 | `hidden`, `reason` | 清單已下架（附原因）/ 已恢復 |
| `account.suspended` / `account.restored` | 當事人 | | 帳號停權 / 恢復 |
| `org.permit_reviewed` | 組織 owner | `org_id`, `display_name`, `permit_no`, `note` | 勸募許可核准（退件沿用 #103 的 `org.permit_rejected`） |
| `org.event_hidden` | 組織 owner | `org_id`, `display_name`, `event_title`, `reason` | 事件下架，所屬據點已暫停 |
| `auth.mfa_enabled` / `auth.recovery_regenerated` | 當事人 | `to`, `provider: "totp"` | 帳號安全通知（比照 `auth.password_changed`，走 `identities::notify`） |
| `staff.digest` | 所有 staff | `reports`, `refunds`, `orgs`, `permits`, `orders`, `failed` | 每日營運待辦彙總，連 `/admin` |

---

## Task 1：基礎設施（先做，序列）

檔案：`db/migrations/0048_notification_center.sql`、`apps/api/src/notify.rs`、新 `apps/api/src/inbox.rs`、`lib.rs`、`account.rs`、`identities.rs`、`auth.rs`、`birthday.rs`、`claims.rs`（僅註解不動 SQL）、`org_members.rs`、`wishlists.rs`（僅 1936–1941 的 raw insert）、`admin.rs`、`tests/smtp_worker.rs` 等受影響測試。

**Migration**
- `notifications` 加 `inbox_at timestamptz`、`inbox_title text`、`inbox_link text`、`read_at timestamptz`；partial index `(user_id, inbox_at DESC) WHERE inbox_at IS NOT NULL`。
- `wishlists` 加 `event_summary_sent_at timestamptz`。
- `users.notification_prefs` 預設改 `'{}'`；既有資料移除 `email_claims`，`email_birthdays` 改名 `email_social`。

**Helpers（notify.rs，全部泛型 `PgExecutor`，同 `enqueue_donors`）**
- `enqueue(ex, user_id, kind, payload)`
- `enqueue_claim_confirmation(ex, claim_id)`：會員認領、`scheduled_at = now() + 10 minutes`（讓連續認領能合併）。
- `enqueue_owner(ex, wishlist_id, kind, payload)`：個人清單給 `owner_id`；組織清單給未停用的 owner / coordinator。payload 併入 `wishlist_id`。
- `enqueue_org_owners(ex, org_id, kind, payload)`：未停用的 owner，帳號未刪除。
- `enqueue_site_paused(ex, wishlist_ids: &[Uuid])`：對各據點 reserved / purchased 認領者與 pledged 認捐者（會員）每人每據點一筆。
- 把 `org_members.rs:189`、`wishlists.rs:1936-1941` 的簡單 raw INSERT 改用 helper。CTE 內的 INSERT 留原處（改動有鎖序風險）；政策集中在 worker，所以不影響功能。

**Worker `tick` 流程（順序固定）**
1. 領取：`attempts + 1`，租約 `scheduled_at = now() + 6 minutes`（須大於 20 封 × 15 秒）。
2. 安靜時段：`category` 為 Gifts / Social 且非 `wishlist.closing`，台北時間 21:00–09:00 → `scheduled_at` 改下一個 09:00、`attempts - 1`、continue。純函式 `quiet_until(now)`，附單元測試。這也修掉生日信半夜寄。
3. 既有的寄前重驗。新增：`claim.confirmation` 認領須仍為 reserved / purchased，否則取消；`friend.accepted` 查 `from_name`，查不到取消。
4. 驚喜把關：`owner.*`、`claim.cancelled_by_donor`、`item.reopened`、`event.summary`：SQL 判 `surprise_locked(w.surprise_mode, w.event_date)` 為真 → `cancelled`、`last_error='surprise locked'`，不進站內信。這同時修掉 `item.reopened` 洩漏驚喜。
5. 合併：kinds = `claim.confirmation`、`claim.purchase_reminder`、`claim.expired`、`item.reopened`。把同收件人、同 kind、`status='pending'`、`scheduled_at <= now()+6 hours`、id 不同的列 UPDATE 成 `status='sent', sent_at=now(), last_error='bundled:<主列 id>'`，RETURNING id 與品項。`claim.purchase_reminder` 的合併條件要含 claim 仍 reserved 且 `expires_at` 未變；`claim.confirmation` 要含 claim 仍有效。把回傳 id 放進 HashSet，同批後續列若在內就跳過。品項標題放 `payload["more_items"]`。
6. render 後，若有 `user_id`：寫 `inbox_at = COALESCE(inbox_at, now())`、`inbox_title`（subject 去掉 `【Wishly】` / `[Wishly] ` 前綴）、`inbox_link`（新純函式 `inbox_link(kind, &payload)` 回相對路徑）。收件人沒有 email、偏好關閉也要先寫站內信，所以 email 取得與判斷要移到這步之後。
7. 偏好：Gifts + `email_gifts=false`、Social + `email_social=false` → `status='cancelled', last_error='muted'`，站內信保留。
8. 寄送；失敗時 `attempts >= MAX_ATTEMPTS` → `failed` 並 `tracing::error!`；否則 `scheduled_at = now() + backoff(attempts)`。`MAX_ATTEMPTS = 6`，backoff 1m / 5m / 30m / 2h / 6h。

**Render**
- 新增上表全部 kinds 的四語文案（zh / en / vi / id），風格與現有一致。刪除 `event.reminder`、移除 `claim.confirmation` 的 `recovery_token` 與 guest 恢復權杖寄送邏輯（`#r=` 前端保留）。
- `more_items` 非空時：主旨加「（共 N 件）」等四語字尾，內文加列表。
- `payload["unsubscribe_url"]`：Gifts → token `'c'`；Social → token `'b'`；訪客 `'g'` 不變。Account 不附。
- 單元測試：`KINDS` 常數列出所有 kind，迴圈檢查四語都不會落到預設「通知」主旨，且 `category` 正確。

**退訂與偏好（account.rs、auth.rs）**
- `verify` 接受 `'c' | 'g' | 'b'`（移除 `'u'`）。`'b'` 設 `email_social=false`、`'c'` 設 `email_gifts=false`；pending 不用取消（worker 會靜音）。
- `PATCH /me` 只接受布林的 `email_gifts` / `email_social`。
- `birthday.rs` 與 worker 內 `email_birthdays` 全改 `email_social`。

**站內通知 API（inbox.rs，CurrentUser，只回本人）**
- `GET /me/notifications?before=<id>&limit=<1..50, 預設 20>` → `{ items: [{id, kind, title, link, created_at, read_at}], next_cursor, unread }`，依 `inbox_at DESC, id DESC`；只看 `inbox_at IS NOT NULL`。
- `GET /me/notifications/unread-count` → `{ unread }`
- `POST /me/notifications/{id}/read` → 204（非本人 404）
- `POST /me/notifications/read-all` → 204

**隱私**：`account.rs:402` 與 `identities.rs:241` 帳號刪除時一併清 `inbox_at / inbox_title / inbox_link`；`GET /me/export` 納入站內通知標題。

**營運**：`POST /admin/notifications/requeue`（Staff、寫 audit）把最近 7 天 `failed` 且 `last_error <> 'bad address'` 的列改回 pending、`attempts=0`、`scheduled_at=now()`，回 `{ requeued }`。

驗收：`cargo test --test notify_foundation`、`--test smtp_worker` 綠；`cargo build` 無警告；上述每個行為至少一個測試（安靜時段延後、驚喜取消、合併、backoff 與 failed、muted 仍進站內信、無 email 仍進站內信、API 只看得到本人、requeue）。

---

## Task 2：送禮與社交發送點

檔案：`claims.rs`、`friends.rs`、`tests/notify_gifts.rs`。

- 認領成功（`create`，會員）→ `enqueue_claim_confirmation`。
- `apply`：`by_owner` 且轉 `cancelled` → 通知捐助者 `claim.cancelled_by_owner`（會員用 `enqueue`；舊訪客沒有 user_id 時比照既有 guest 分支寫 guest 通知）。
- `apply`：本人從 `purchased` 轉 `cancelled` → `enqueue_owner(... 'claim.cancelled_by_donor' ...)`。從 `reserved` 取消不通知。
- `friends.rs` `accept` 與 `invite_accept`：通知另一方 `friend.accepted`。

## Task 3：金流發送點

檔案：`topup.rs`、`wallet_ops.rs`、`orders.rs`、`contributions.rs`、`tests/notify_money.rs`。

- `topup.rs::credit`：實際入帳時 `topup.credited`（重送 webhook 不得重複）。
- `topup.rs::expire_due`：只對 `payment_info IS NOT NULL`（已取號的超商 / ATM）寄 `topup.expired`，沒取號的放棄訂單不寄。
- `cash_refund_patch`：轉 `completed` / `rejected` 時 `cash_refund.updated`；重送不重複。
- `orders.rs::set_wallet_status`：`wallet.frozen` / `wallet.unfrozen`，狀態真的改變才寄。
- `dispute_open` → `wallet.frozen`；`dispute_won` → `wallet.unfrozen`（確實解凍才寄）；`charge_back` → `wallet.chargeback`（`amount` 扣回點數、`debt` 新欠款）。都要冪等。
- `contributions.rs::on_funded` → `enqueue_owner('owner.funded')`。
- `tick_funding` 自動退點：`points::release` 之前 `enqueue_donors(item, "funding.released", &["pledged"])`；`rel.is_empty()` 的 continue 會 rollback，無需處理。
- `orders.rs::patch_order`：shipped / delivered 加 `owner.shipped` / `owner.delivered`；failed / cancelled 加 `owner.order_failed`；placed 且 `refunded > 0` 加 `enqueue_donors(item, "order.partial_refund", &["captured"])`。

## Task 4：治理、組織、安全與排程

檔案：`reports.rs`、`admin.rs`、`org_admin.rs`、`relief_events.rs`、`org_sites.rs`、`orgs.rs`、`wishlists.rs`（僅據點暫停發送點）、`security.rs`、新 `apps/api/src/notify_jobs.rs`、`ratelimit.rs`、`lib.rs`、`tests/notify_governance.rs`。

- `reports.rs::moderate`：實際更新時通知擁有者 `wishlist.moderated`（`enqueue_owner`，鎖定中也要送達：此 kind 不在驚喜把關名單）。
- `admin.rs::update_user`：`account.suspended` / `account.restored`，只在 blocked 真的改變時寄。
- `org_admin.rs::review_permit`：pending → approved / rejected 寄 `org.permit_reviewed`；撤銷已核准的沿用 `org.permit_revoked`。
- `relief_events.rs::moderate`：下架時，對有據點被暫停的組織寄 `org.event_hidden`。
- 找出所有把據點改成 `paused` / `closed` 的路徑（`rg "site_status"`），含許可到期、許可撤銷、組織撤銷、事件下架、擁有者手動暫停，全部呼叫 `enqueue_site_paused`。
- `security.rs`：`totp_enable`、`regenerate_recovery` 呼叫 `identities::notify`（`auth.mfa_enabled` / `auth.recovery_regenerated`）。worker 的 `security` 判斷若需加這兩個 kind，改 `notify.rs` 該處一行即可（本任務允許這一處）。
- `notify_jobs.rs::event_summary_tick(pool)`：`event_date` 落在台北今天往前 7 天內、`event_summary_sent_at IS NULL`、未鎖定、`org_id IS NULL`、清單未刪除，且有 reserved / purchased / delivered 認領或 captured 認捐者 → 條件式 UPDATE 取得唯一性後寄 `event.summary`，`givers` 為去重人數（user_id + guest_id）。多實例安全。
- `notify_jobs.rs::staff_digest_tick(pool)`：有待辦才寄，給每位 `is_staff`、未封鎖帳號；同一人 20 小時內已有 `staff.digest` 就略過。待辦：open reports、requested cash refunds、待審組織、pending permits、待處理 orders、`failed` 通知數。
- `ratelimit.rs` 既有每小時分支呼叫這兩個 tick。

## Task 5：前端與文件

檔案：`apps/web` 與 `docs/`、`CLAUDE.md`。

- `CreatorHeader` 加鈴鐺與未讀徽章（`GET /me/notifications/unread-count`，掛載與換頁時取一次，不輪詢、不走 SSE）；無障礙標籤四語。樣式走 `creator.css` token。
- 新頁 `pages/me/notifications.vue`：列表、未讀標示、點擊標已讀並導向 `link`、全部已讀、載入更多、空狀態。`/me/notifications` 加進 `nuxt.config.ts` 的 `ssr: false` 清單。
- `settings.vue`：通知區改三列：帳務與安全（固定開啟、disabled）、送禮進度 `email_gifts`、社交與生日 `email_social`。
- `guest.unsub.*` 文案改通用（「不再收到這類通知信」），四語。
- `me.claims.confirmCancelPurchased` 四語改為反映驚喜期間不通知：「建立者會收到通知（驚喜期間不會）。確定不送了嗎？」
- 後台 `admin/index.vue`：`notifications_by_status.failed > 0` 時顯示重新排入按鈕，呼叫 `POST /admin/notifications/requeue`。
- 訪客 mock 不動。新邏輯有 vitest（若有非平凡 util）；`npm run typecheck && npm test && npm run build` 綠。
- 文件：`docs/01-prd.html` 加決策 D67（通知三分類、站內通知中心、安靜時段、重試 6 次）；`docs/04-data-schema-api-contract.html` 更新 notifications 欄位、prefs 鍵、四個新 API 與 requeue；`docs/flow-verification/decisions-data.js` 的「最多 3 次」改註明被 D67 取代；`CLAUDE.md` 架構段加通知一條（registry、worker 順序、新增 kind 要改哪裡）。
