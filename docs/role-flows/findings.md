# 角色流程模擬：發現、處置與目前缺失（2026-10-29）

> 模擬執行於 2026-10-29 的 main（89ee004）。之後 main 合併了 #83（客服信箱連結）、#86（新手導覽）、#102（許可續期，關閉 #87），本分支已合併並重跑測試；各角色文件保留當時的觀察。

本頁彙整 [01 受捐者](01-user-recipient.md)、[02 捐助者](02-user-donor.md)、[03 公益組織](03-org.md)、[04 營運者](04-operator.md) 四份實機模擬的結果。每個問題的重現步驟、截圖檔名、相關程式碼都在各自文件；這裡只記「結論與處置」。

## 1. 執行總覽

| 角色 | 步驟 | ✅ | ⚠️ gap | ❌ fail | ⛔ blocked | 腳本 |
|---|---|---|---|---|---|---|
| 受捐者 | 67 | 54 | 6 | 1 | 6 | `apps/web/qa/role-flows/recipient.mjs` |
| 捐助者 | 72 | 63 | 4 | 0 | 5 | `apps/web/qa/role-flows/donor.mjs` |
| 公益組織 | 62 | 47 | 15 | 0 | 0 | `apps/web/qa/role-flows/org.mjs`、`org-tick.mjs` |
| 營運者 | 68 | 57 | 11 | 0 | 0 | `apps/web/qa/role-flows/operator.mjs`（P-92 另以 `FLOW_ONLY=J FLOW_READONLY=1` 單獨執行） |

各數字為最後一次完整執行（修正前的產品程式碼）；❌／⚠️ 的處置見下。

**核心機制沒有發現錯誤**：防超賣（2 人搶最後一件、6 人搶 1 件、同一人不同 Idempotency-Key 同時送 3 次）、冪等重放、逾期釋放（約 2 分鐘內由背景工作釋放並寄信）、眾籌扣點與按比例退回、綠界儲值入帳、錢包對帳不變式（Σ lots = balance = Σ ledger，全庫 0 筆不一致）、MFA 重放拒絕、組織申請狀態機、權限（非成員 404、volunteer 403）皆符合預期。

## 2. 已在本分支修正

| 編號 | 問題 | 修正 | 驗證 |
|---|---|---|---|
| R-P1 | 分享頁「結束時間」SSR 以 UTC 輸出，與瀏覽器差 8 小時並 hydration mismatch | `pages/s/[slug].vue` 固定 `timeZone: 'Asia/Taipei'`（與全站慣例一致） | typecheck、瀏覽器重測 |
| R-P3 | 已達標進入採購的眾籌品項，刪除對話框仍提供「連同認領一併刪除」（API 永遠 409） | `lists/[id]/edit.vue`：已達標時顯示後端原因、不提供強制刪除 | typecheck |
| D-P1 | 捐助者頁面（分享頁、我的送禮、探索…）沒有帳號選單，無法登出、找不到錢包 | `GuestTop` 在瀏覽器端補上 `CreatorUserMenu`；只在有登入旗標時才打 `/me`，SSR／CDN 快取仍為匿名版 | 單元測試 2 則 |
| D-P2 | `/me/claims` 未登入只有空狀態 | 空狀態加「登入」並帶回原頁 | typecheck |
| D-P3 | 退訂頁一律寫「送禮通知」，但生日通知信也用同一頁 | 依權杖種類（`b:`）顯示生日通知文案（四語） | typecheck |
| D-P4 | 搶輸最後一件的提示只閃 1.2 秒 | 分享頁留下 toast（編輯既有認領時不顯示） | typecheck |
| O-P2 | 許可過期後最長 1 小時內，據點仍公開並接受認捐（要等每小時 tick） | `claims.rs` `SITE_STATUS_SQL`：認捐、加量、眾籌贊助／轉投都把「上線中但許可已過期」視同暫停；`public.rs` `LIVE_SITE` 排除過期許可。鎖序不變 | `relief_items::expired_permit_blocks_site_before_tick`；mutant `claim-wishlist-for-share` 更新後仍 killed |
| O-P4（前端） | 組織被撤銷後，後台照常顯示「新增事件／據點」 | 非 verified 時顯示橫幅（含審核備註、連到申請頁）並隱藏新增入口 | typecheck |
| O-P5 | 組織看不到認捐數 | 據點品項顯示「累計已認領 n」（含已到貨；在途數需另由認捐狀態推導，見 PR #103 討論） | typecheck |
| O-P6 | 申請表許可日期錯誤被丟回第 1 步 | `ptrStep` 將許可欄位 pointer 對應第 3 步 | typecheck |
| O-P7 | 組織文件只看 `Content-Type`，改副檔名的純文字檔也收 | `orgs.rs` 比對 PDF／JPEG／PNG 檔頭，不符回 422 `MISMATCH` | `org_documents` 測試 |
| O-P8a | `/org` 介紹頁寫「可開捐款收據」，功能不存在（D63 延後） | 改為「捐款收據（即將推出）」，SEO 描述移除（四語） | — |
| O-P8e | 系統手冊仍寫 relief「規劃中、API 回 422」、「點數無儲值管道」 | 更新 `docs/user-manuals/system-handbook.html` | — |
| O-P9 | 據點徽章顯示英文 `paused`、active 顯示「進行中」 | `StatusBadge` 加 `label`；據點用 `org.relief.siteStatus.*` | typecheck |
| O-P11 | 救災據點的眾籌截止標「選填」，實為必填 | relief 時標「必填」並設 `required` | typecheck |
| O-P17 | 核准信印「審核備註：-」；重新送審保留上一輪備註 | 無備註不印該行；送審清除 `review_note` | `org_review`、`org_apply` 測試 |
| P-P3 | 送審中的許可被退件，申請人沒收到信 | 新通知 `org.permit_rejected`（四語，含備註） | `org_review::reject_pending_permit_notifies_owner` |
| P-P10 | staff 下載組織文件的稽核記為 `user` | 經 staff 身分下載時改記 `actor_type=staff` | `org_documents` 測試 |
| P-P11 | 事件下架原因後端必填，UI 未擋 | UI 必填與前端檢查（四語） | typecheck |
| P-P15 | 非 staff 進 `/admin` 顯示「403」大字配「找不到頁面」 | 大字改顯示 404，與「看起來像不存在」的設計一致 | typecheck |
| P-P17（前半） | 組織詳情的組織類型顯示 `foundation` | 顯示 i18n 標籤 | typecheck |
| （合併 main 時發現） | main 上 #86 的 `0044_onboarding.sql` 與 #102 的 `0044_permit_reminders.sql` 版本號重複，任何新資料庫套 migration 都失敗（`_sqlx_migrations_pkey` 重複 44） | 後合併的 `permit_reminders` 改為 `0045_permit_reminders.sql` | 整包 `cargo test` |
| （PR CI 發現，main 同樣存在） | `org_sites::org_site_images_survive_creator_deletion` 約 1/10 間歇失敗：刪帳號在交易內已寫入圖片退役要求，提交後 `delete_objects` 又在背景重寫一次；drain 判定「據點仍引用」撤銷要求後，重寫把要求掛回去 | `delete_me` 改呼叫新的 `uploads::drain_soon`（只在背景處理待辦、不重寫要求） | 本機 org_sites 連跑 15 次 0 失敗（修正前 main 10 次 1 失敗） |
| R-P7、R-P8 | PRD US-38／US-12、線框 5.28 仍寫「有人捐助時寄信」；線框寫拖曳排序 | 標註 D58 已停用；排序改為 ↑↓ | — |

驗證指令：`cargo test`（合併 main 後 86 個測試檔、536 則全過）、`cargo clippy --all-targets`、`cargo fmt --check`、`npm run typecheck`、`npx vitest run`（302 則全過）、`scripts/mutate.sh all`（92／92 killed）。

## 3. 開立的 issue（需要產品／設計決定，或範圍超出本次修正）

| Issue | 主題 | 來源 |
|---|---|---|
| [#87](https://github.com/erictheng93/wishly/issues/87) | ~~組織通過後沒有畫面可新增／續期勸募許可~~ → 已由 #102 實作並關閉（本分支已合併 main） | O-P1 |
| [#88](https://github.com/erictheng93/wishly/issues/88) | 成員邀請對未註冊 email 不寄信 | O-P3、O-P8f |
| [#89](https://github.com/erictheng93/wishly/issues/89) | 組織／據點狀態變更的通知範圍與原因可見度（coordinator、在途捐助者、事件下架） | O-P4、O-P10、O-P14、P-P2 |
| [#90](https://github.com/erictheng93/wishly/issues/90) | 實收登記：撤銷後仍可登記、無更正／作廢、超收無提示 | O-P15、O-P16 |
| [#91](https://github.com/erictheng93/wishly/issues/91) | 組織功能與規格落差（審核中預備、地圖座標、Logo、統編提早檢查、離開／轉讓） | O-P8b/c/d、O-P12、O-P13 |
| [#92](https://github.com/erictheng93/wishly/issues/92) | 全站沒有客服管道，錯誤訊息卻叫人聯絡客服 → #83 已加客服信箱連結（模擬時尚未合併）；後台工單佇列仍缺 | P-P1 |
| [#93](https://github.com/erictheng93/wishly/issues/93) | 營運動作沒有通知當事人（退款、凍結、下架、停權、採購單進度給受捐者） | P-P4、P-P5 |
| [#94](https://github.com/erictheng93/wishly/issues/94) | 誤撤銷組織無法復原；退件／撤銷備註不強制 | P-P6、P-P13 |
| [#95](https://github.com/erictheng93/wishly/issues/95) | 後台首頁沒有待辦計數；分析頁缺營運／金流指標 | P-P7、P-P16 |
| [#96](https://github.com/erictheng93/wishly/issues/96) | 沒有系統公告／維護通知 | P-P8 |
| [#97](https://github.com/erictheng93/wishly/issues/97) | `GET /admin/orders` 每次載入都寫 `order.view_address` 稽核 | P-P9 |
| [#98](https://github.com/erictheng93/wishly/issues/98) | 後台查詢細節（儲值搜尋與 Email 遮罩、檢舉結案備註與累計、許可對應文件） | P-P12、P-P14、P-P17 後半 |
| [#99](https://github.com/erictheng93/wishly/issues/99) | **安全**：高風險後台動作未強制 MFA；任一 staff 可重設他人 MFA | P-P18 |
| [#100](https://github.com/erictheng93/wishly/issues/100) | 「顯示送禮者暱稱」預設無效；匿名與可見度控制重疊 | R-P2、D-P5 |
| [#101](https://github.com/erictheng93/wishly/issues/101) | 參考價格不在分享頁顯示、取消他人認領無通知與原因、沒有謝卡 | R-P4、R-P5、R-P6 |

## 4. 各角色目前缺少、但應該有的能力

詳列於各文件（03 §7、04「營運角色缺少的能力」）。摘要：

- **受捐者**：感謝／謝卡、取消他人認領時的溝通（#101）。
- **捐助者**：名稱顯示設定直覺化（#100）；被受捐者取消時的通知（#101）。
- **公益組織**：許可管理（#87，已由 #102 完成）、組織資料維護、捐款收據（D63）、在途與缺口儀表板（本分支已加「已認領」）、對捐助者的回報與成果公開、實收更正與簽收憑證（#90）、事件編輯、成員自助（#91）、組織內操作紀錄畫面。
- **營運者**：客服工單（#92；客服信箱連結已由 #83 補上）、公告（#96）、待辦計數與營運分析（#95）、組織恢復（#94）、批次操作（批次核准發點、批次處理檢舉）、MFA 強制（#99）。

## 5. 阻擋與本環境無法驗證的項目

### 5.1 環境阻擋（已繞道，記錄以便重現）

| 阻擋 | 處置 | 影響 |
|---|---|---|
| Docker daemon 未啟動 | 手動 `dockerd` | 無 |
| Docker Hub 匿名拉取 429（mailpit、minio 映像） | Mailpit 改用 GitHub release 執行檔；PostGIS 映像拉取成功 | 無 |
| `dl.min.io` 被出口代理拒絕 | S3 改用 `moto_server` | 圖片上傳後讀取 403、畫面破圖；**圖片流程（去 EXIF、條件式 PUT）未驗證** |
| 專案 Playwright 1.64.1 對應的 Chromium 版本不在沙盒 | `lib.mjs` 優先用 `/opt/pw-browsers/chromium` | 無 |
| `mutate.sh all` 後 `apps/api/target` 累積約 27 GB，整包 `cargo test` 連結時磁碟滿（`ld` Bus error） | `cargo clean` 後以 `CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0` 重跑 | 無；在磁碟配額有限的環境跑突變測試後應先清理 |
| 真實綠界無法連線 | `ecpay-sim.mjs`：QueryTradeInfo 替身＋簽章付款通知 | 取號（ATM／超商代碼）`/info` 通知、真實退款與拒付 webhook 未驗證 |

### 5.2 需要外部條件或真機（⛔）

- Google／LINE 登入（按鈕回「此登入方式尚未開放」）、Turnstile 人機驗證：需正式憑證（既有 TODO #6）。
- LINE 內建瀏覽器、iOS WebKit：只以 UA 字串模擬（既有 P0 Spike S2）。
- Cloudflare CDN 快取與 purge：本機無 CDN（`cdn::purge_*` 只驗證到呼叫）。
- 原生殼（Tauri）、原生分享面板。
- 背景排程：`permit_expiry_tick` 實際等到兩次整點 tick（16:51、17:51）驗證據點自動暫停並寄信給 owner；生日通知的寄送時間未等待。
- 「最後一位 staff 保護」：共用環境有其他 staff，無法觸發，只讀碼確認。

### 5.3 延後執行

- **P-92 全站唯讀模式**：會讓同時執行的其他角色寫入 503，平行模擬時跳過；四個角色跑完後單獨執行，結果見 §6。

## 6. 收尾驗證

| 項目 | 結果 |
|---|---|
| P-92 全站唯讀模式（`FLOW_ONLY=J FLOW_READONLY=1 node qa/role-flows/operator.mjs`） | ✅ 開啟後一般寫入回 503 `READ_ONLY_MODE`、後台旗標仍可操作；關閉後 `/public/status` 回 `read_only: false` |
| 修正後的瀏覽器重測（`node qa/role-flows/verify-fixes.mjs`，Web 以本分支重新建置） | ✅ 7／7：R-P1（台北時間、無 hydration 警告）、D-P1、D-P2、D-P3、D-P4（先開 sheet 再被搶走；SSE 也會即時把品項改成已認領完）、P-P15、O-P4 |
| API 端修正（O-P2、O-P7、O-P17、P-P3、P-P10） | 以 `cargo test` 新增／更新的整合測試驗證（見 §2） |
| `scripts/mutate.sh all` | ✅ 92／92 killed（含更新後的 `claim-wishlist-for-share`），結束後原始碼還原、工作樹乾淨 |

另：分享頁對未登入訪客有 2 個 401 console 訊息（分享頁自身查詢登入狀態），修正前的捐助者模擬（D-01）就已存在，非本分支造成。
