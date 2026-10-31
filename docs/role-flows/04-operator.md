# 04 平台營運者（staff）流程設計與執行結果

> 執行日期 2026-10-29｜腳本 `apps/web/qa/role-flows/operator.mjs`｜結果 `apps/web/qa/out/role-flows/operator.json` 與 `operator/*.png`（不進 git）
> 重現：`cd apps/web && node qa/role-flows/operator.mjs`（`FLOW_ONLY=A,B` 只跑區段；`FLOW_READONLY=1` 才跑全站唯讀步驟）

## 1. 角色設定

同一個後台，五種工作內容。每個「人」是一個獨立帳號與瀏覽器情境（cookie 隔離）；帳號先註冊、再以 SQL 設 `is_staff`（平台外動作，系統手冊 6.1 也規定第一位 staff 必須這樣建立）。

| 角色 | 帳號 | 主要頁面 | 關心什麼 |
|---|---|---|---|
| 營運主管 | `lead`（啟用 TOTP） | `/admin`、`/admin/orgs`、`/admin/relief-events`、`/admin/analytics`、`/admin/audit`、`/admin/mfa` | 組織與許可是否可信、事件是否重複、KPI、稽核 |
| 客服 | `cs`（不啟用 MFA，用來幫主管重設） | `/admin` 檢舉／清單／使用者分頁 | 檢舉處理、下架與還原、停權、帳號復原、重設 MFA、客訴窗口 |
| 金流對帳 | `fin` | `/admin/topups`、`/admin/cash-refunds`、`/admin/wallet-debts`、`/admin/wallets` | 爭議／拒付、現金退款、欠款結案、人工發點、對帳不變式 |
| 送禮執行人 | `ship` | `/admin/orders` | 眾籌達標後代買代送：下單 → 出貨 → 送達、問題單、收件資訊 |
| 品牌經理 | `brand` | `/admin/catalog`、公開 `/brands` | 品牌與精選商品上下架、對外溝通 |

另有 1 位一般用戶（驗證權限邊界）、被檢舉清單的擁有者、申請組織的人、受捐者、捐助者、付款人等「對手方」，全由 API 建立（前置準備，標為平台外）。

## 2. 目標

1. 營運者每一步都能用 UI 完成，且後端狀態、通知、公開頁連動與預期一致。
2. 敏感動作（讀收件明文、發點、停權、撤銷）一律有二次確認、必填原因、稽核。
3. 金流不變式（`balance = Σ lots = Σ ledger`）在任何營運操作後都成立。
4. 找出營運者「想做卻做不到」或「做了對方不知道」的缺口。

## 3. 流程總覽

| 區段 | 內容 | 步驟 |
|---|---|---|
| A 進後台 | 一般用戶被拒、未登入導向、首次登入、MFA 設定、登出再登入過 MFA | P-01 ~ P-04 |
| B 組織審核 | 待審清單、看文件、要求補件（含通知信）、補件重送、許可核准／退件、核准／退件組織 | P-10 ~ P-18 |
| C 救災事件 | 事件下架（含據點暫停）、取消下架、合併重複事件 | P-20 ~ P-25 |
| D 撤銷連動 | 撤銷許可、撤銷組織與公開頁連動、誤撤銷復原 | P-30 ~ P-32 |
| E 送禮執行 | 採購單佇列、下單（差額退回）、出貨、送達、失敗、取消、並行衝突、通知 | P-40 ~ P-48 |
| F 金流營運 | 儲值查詢、爭議／勝訴／敗訴扣回、欠款結案、現金退款、人工發點／扣點／凍結、對帳 | P-50 ~ P-5A |
| G 內容治理與客服 | 檢舉佇列、下架／還原、駁回、使用者停權、復原信、staff 授權、客訴通道 | P-60 ~ P-69 |
| H 品牌 | 新增品牌／商品、Logo 上傳、公開頁顯示、下架、公告能力 | P-70 ~ P-74 |
| I 數據與稽核 | KPI 對照、待辦總覽、分析頁、稽核查詢與涵蓋 | P-80 ~ P-85 |
| J 其他 | 手機版、MFA 遺失重設、全站唯讀模式 | P-90 ~ P-92 |

模擬原則同 `README.md`：能用 UI 就用 UI；平台外（建前置資料、收信、金流付款通知、清限流計數）才走 API／SQL／替身並在步驟註明。限流：同一 IP 每小時 10 筆檢舉（`report_ip`）多次重跑會累積，P-60 開頭會清掉 `rate_limits` 的 `report_*` 計數（平台外）。

## 4. 步驟設計（ID、角色、操作與預期）

| ID | 角色 | 操作與預期結果 |
|---|---|---|
| P-00 | （前置） | 前置準備：建立五位營運人員與一般用戶（平台外：註冊 + SQL 設 is_staff） |
| P-01 | 營運主管 | 一般用戶進 /admin 被拒（UI 與 API 皆然） |
| P-01b | 營運主管 | 未登入者進 /admin：導向登入頁並帶 redirect |
| P-02 | 營運主管 | 營運主管首次登入進後台：導向登入 → 回到 /admin，看到 STAFF 徽章、KPI 與建議啟用 MFA 提示 |
| P-03 | 營運主管 | MFA 設定：現在設定 → 開始設定 → 讀金鑰 → 錯誤碼被拒 → 正確 TOTP 啟用 → 回後台、提示消失 |
| P-04 | 營運主管 | 登出再登入：必須先過 MFA（重放舊碼被拒、下一個時間步的新碼通過） |
| P-10 | 營運主管 | 前置：兩個申請人各送一份組織申請（A 有 2 張許可、B 有 1 張）（平台外：申請端已由 03-org 覆蓋，這裡用 API 建立） |
| P-11 | 營運主管 | 待審清單：預設只看待審核、先送先審；檢視詳情看到欄位、文件、許可、成員 |
| P-12 | 營運主管 | 檢視文件：下載立案文件（已加密儲存，下載時解密）內容與原檔一致 |
| P-13 | 營運主管 | 要求補件：未勾欄位被擋；勾「法定名稱＋文件」並填理由 → 狀態變需補件，申請人收到通知信 |
| P-14 | 營運主管 | 申請人補件後重新送審（平台外：申請端），營運者在待審清單再次看到 |
| P-15 | 營運主管 | 許可審核：A 的許可 1 核准、許可 2 退件（含備註） |
| P-16 | 營運主管 | 許可退件是否通知申請人（理由要能讓對方知道怎麼改） |
| P-17 | 營運主管 | 核准組織：B 沒有核准許可時被擋（PERMIT_REQUIRED）；A 核准後通知申請人、公開組織頁可見 |
| P-18 | 營運主管 | 退件：B 填備註退件 → 申請人收到信、30 天內不能重新申請 |
| P-20 | 營運主管 | 前置：組織 A 建立災害事件 EvA 與重複事件 EvDup，各掛一個上線據點（平台外：組織端已由 03-org 覆蓋） |
| P-21 | 營運主管 | 事件管理列表：以 slug 篩選找到事件（列表來自公開 API，無 admin 列表端點） |
| P-22 | 營運主管 | 下架事件：不填原因被後端擋；填原因後事件隱藏、掛在其下的據點被暫停、公開頁消失 |
| P-23 | 營運主管 | 事件下架是否通知所屬組織（owner 要知道據點為何被暫停） |
| P-24 | 營運主管 | 取消下架：事件回到公開，但已暫停的據點不會自動恢復 |
| P-25 | 營運主管 | 合併重複事件：EvDup 合併到 EvA（自己合併自己被擋）→ 據點改掛 EvA，EvDup 從公開列表消失 |
| P-30 | 營運主管 | 撤銷已核准許可：使用此許可的上線據點被暫停、公開頁與組織頁消失、owner 收到信 |
| P-31 | 營運主管 | 撤銷組織（兩段確認）：據點全部降為草稿、公開組織頁 404、owner 收到「驗證已撤銷」 |
| P-32 | 營運主管 | 誤撤銷的復原路徑：已退件的組織在後台沒有任何「恢復」動作 |
| P-40 | 送禮執行人 | 前置：受捐者建立 4 個眾籌品項（收件資訊已填）、捐助者領點並贊助達標 → 產生 4 張採購單（平台外：API） |
| P-41 | 送禮執行人 | 採購單佇列：預設只看待下單；收件資訊解密顯示並寫稽核；篩選狀態與履約類型 |
| P-42 | 送禮執行人 | 標記已下單：缺訂單編號、金額超過目標被擋；實付 900 → 差額 100 依比例退回捐助者 |
| P-43 | 送禮執行人 | 標記已出貨（物流單號）→ 兩位捐助者收到「已出貨」信 |
| P-44 | 送禮執行人 | 標記已送達 → 品項標為已履行、捐助者收「已送達」信；公開頁顯示已送達 |
| P-45 | 送禮執行人 | 問題單：缺貨 → 標記失敗（原因必填）→ 點數全額退回、品項回到募集中、捐助者收信 |
| P-46 | 送禮執行人 | 先下單（實付 250，退差額 50）再取消採購 → 已扣用的 250 全數退回，捐助者淨變動為 0 |
| P-47 | 送禮執行人 | 兩位承辦同時處理同一張單：後送出的人看到「狀態已被更新過」並重讀列表 |
| P-48 | 送禮執行人 | 受捐者（清單擁有者）在採購過程中是否收到任何通知 |
| P-50 | 金流對帳 | 前置：5 位付款人（儲值已入帳／領點）與 3 筆現金退款申請（平台外：API 與綠界替身） |
| P-51 | 金流對帳 | 儲值查詢：預設「爭議中」；切到「已入帳」找到 p2 的單（無搜尋欄，需逐頁找） |
| P-52 | 金流對帳 | 標記爭議 → 錢包被凍結（列上顯示「錢包已凍結」）；凍結中不能贊助 |
| P-53 | 金流對帳 | 爭議勝訴 → 儲值單回「已入帳」、錢包自動解凍 |
| P-54 | 金流對帳 | 拒付敗訴（部分已花掉）：p1 儲值 1000 已贊助 600 → 扣回 400、記欠款 600、錢包保持凍結 |
| P-55 | 金流對帳 | 拒付欠款：p1 欠 600；備註必填 → 標記已追回；p4（儲值全花光）敗訴後欠 300 → 核銷 |
| P-56 | 金流對帳 | 現金退款：申請 200 → 開始處理 → 標記已退款（不可逆） |
| P-57 | 金流對帳 | 現金退款：拒絕並退點（理由必填）→ 點數退回原批次；使用者可看到拒絕理由 |
| P-58 | 金流對帳 | 現金退款處理結果是否通知使用者（完成匯款／被拒絕） |
| P-59 | 金流對帳 | 點數錢包：搜尋 → 發點 +500（adjustment）→ 扣點 -200 → 扣到負數被擋 → 凍結／解凍（原因必填） |
| P-5A | 金流對帳 | 對帳不變式（SQL）：每個錢包 balance = Σ lots.remaining = Σ ledger.delta；退款申請單狀態與分配守恆 |
| P-60 | 客服 | 前置：被檢舉的 3 份清單（詐騙／不當內容／著作權）與一位待客服處理的用戶（平台外：API） |
| P-61 | 客服 | 檢舉佇列：看到檢舉理由／補充說明／檢舉者類型；首頁 KPI「待處理檢舉」與資料庫一致 |
| P-62 | 客服 | 下架清單：原因必填（空白時按鈕 disabled）→ 公開頁 410、檢舉自動結案、KPI 更新、創建者儀表板看到原因 |
| P-63 | 客服 | 下架是否主動通知創建者（信件） |
| P-64 | 客服 | 駁回與結案：清單 2 駁回、清單 3 標已處理（不下架）；切換「已處理／已駁回」可查 |
| P-65 | 客服 | 清單搜尋與還原：以 slug 找到被下架的清單 → 恢復上架 → 公開頁 200 |
| P-66 | 客服 | 使用者管理：查詢 → 顯示 Email（預設遮罩）→ 停權（原因必填）→ 登入端看到停權 → 解除停權 |
| P-67 | 客服 | 帳號復原：寄送復原驗證信 → 用戶收到「重設密碼」驗證碼（後台不能直接改密碼／Email） |
| P-68 | 客服 | 設為 staff／取消 staff（原因必填，撤銷對方現有登入） |
| P-69 | 客服 | 客服／回饋通道：站內找得到聯絡窗口、意見回饋、客訴表單、工單嗎？ |
| P-70 | 品牌經理 | 品牌館：新增品牌（slug、名稱、簡介、官網）→ 列表出現 → 公開 /brands 顯示 |
| P-71 | 品牌經理 | 品牌 Logo 上傳（CreatorImageUploader → S3 替身） |
| P-72 | 品牌經理 | 新增商品（ref、品牌、規格、推薦說明、價格、商品連結、適合對象／場合）→ 品牌頁顯示商品 |
| P-73 | 品牌經理 | 編輯與下架：改價格、重複 slug 被擋、下架品牌後 /brands 不再列出（ref／slug 不可改，無刪除） |
| P-74 | 品牌經理 | 對外公告／系統通知／群發信（品牌溝通）：找得到嗎？ |
| P-80 | 營運主管 | 首頁 KPI：數字與 /admin/stats、資料庫一致；新增資料後「更新統計」會變動 |
| P-81 | 營運主管 | 待辦總覽缺口：首頁不顯示待審組織／待處理採購單／待審退款／爭議儲值的數量 |
| P-82 | 營運主管 | 數據分析 /admin/analytics：各區間可切換；總覽數字與資料庫（排除 staff／已刪除）一致；小樣本以 <5 隱藏 |
| P-83 | 營運主管 | 數據缺口：分析頁沒有金流（儲值／退款／拒付）、採購履約、組織審核、客服處理時效等營運指標 |
| P-84 | 營運主管 | 稽核紀錄：以自己的 UUID 過濾看到剛才的操作；動作／對象／日期過濾；展開操作內容 |
| P-85 | 營運主管 | 稽核涵蓋度：各角色的操作是否都留下稽核（SQL 對照）；staff 下載組織文件的稽核身分 |
| P-90 | 送禮執行人（手機） | 手機版（390 寬）：各後台頁沒有橫向捲動、關鍵按鈕可點 |
| P-91 | 客服／主管 | 手機遺失：客服對主管按「重設兩步驟驗證」→ 主管現有登入被撤銷，重新登入後不再要求 MFA（並被提示重新設定） |
| P-92 | 客服／主管 | 全站唯讀模式：開啟 → 一般寫入回 503、後台旗標仍可操作 → 關閉 |

## 5. 執行結果

**總計 68 步：✅ 56、⚠️ 11、⛔ 1、❌ 0。** 圖例：✅ 依設計走完；⚠️ 能走但有缺口（見第 6 節）；⛔ 本環境不執行；❌ 產品錯誤（本次無）。
調整紀錄：腳本迭代中修過的都是選擇器／等待問題（下拉重載要等請求、`getByLabel` exact 會吃到 option 文字、卡片需先展開），沒有把腳本問題記成產品問題。

## 6. 執行結果

共 **72** 個步驟：✅ pass 63、❌ fail 0、⚠️ gap 4、⛔ blocked 5。

「觀察」欄是腳本在該步驟實際讀到的資料（原樣摘錄，過長者截斷）。截圖在 `apps/web/qa/out/role-flows/donor/`。

| ID | 狀態 | 步驟 | 觀察 | 截圖 |
|---|---|---|---|---|
| D-00 | ✅ | 前置準備：用 API 建受捐者「小愛」與各種狀態的清單、營運帳號 | 標記 T=ec886c47；清單 slug：main=VCiD76YAxL surprise=SpjPCRGpfo friends=69TSzRSnLK private=rAbKX8uFUE closed=nZUDUiazj0 gone=iXsKFfhYJR race=Ul4ZyEmo7Q cf=jlXKnM7px1；受捐者與營運動作皆為前置（API）；gone 清單由營運 API 下架以測 410 | — |
| D-01 | ✅ | 桌面・未登入開分享頁：標題、進度、品項卡、登入引導、SSR OG | og:title=幫小愛ec8挑禮物｜ec886c47 小愛的待產清單｜og:description=已完成 0%，還有 5 項等你來送。瀏覽不用註冊，登入會員後即可認領。｜og:image=http://localhost:3000/og-default.png；footer 顯示「即時更新中」（SSE 已連上）；這個頁面保持開啟當觀察者 | `D-01.png` |
| D-02 | ✅ | 手機 390 寬・LINE 內建瀏覽器 UA：版面、LINE 引導橫幅、點擊目標 | 「登入後來送」按鈕尺寸 112x44px（建議 ≥44px）；LINE UA 為模擬字串（isLineBrowser 只比對 /Line\//）；真機 LINE 內建瀏覽器行為見 D-90 | `D-02.png` |
| D-03 | ✅ | 驚喜清單：訪客看到的提示與捐助者姓名顯示（對照一般清單） | 驚喜清單對捐助者仍可正常認捐；提示文字提醒不要對壽星洩漏。建立者本人看到的遮蔽屬受捐者流程 | — |
| D-04 | ✅ | 品項深連結 ?item=<id>：捲到該卡並加亮 |  | — |
| D-05 | ✅ | 關閉 JavaScript（舊瀏覽器／內嵌）：SSR 內容仍可讀，無送禮按鈕 | 登入連結仍是一般連結，可點 | — |
| D-06 | ✅ | 按「登入後來送」→ 登入頁帶 redirect → 改點「建立帳號」，redirect 仍保留 | 登入頁另有 Google / LINE 登入鈕（第三方登入無法在本環境驗證，見 D-91） | `D-06.png` |
| D-07 | ✅ | 註冊表單驗證：密碼太短、錯誤驗證碼、重寄倒數 | OTP 信主旨「Wishly 註冊驗證碼」；驗證碼取自 Mailpit（平台外動作） | `D-07.png` |
| D-08 | ✅ | 輸入正確驗證碼完成註冊 → 自動回到原清單 /s/slug 並已登入 |  | `D-08.png` |
| D-09 | ⚠️ | 登入後的分享頁／我的送禮頁：找得到帳號選單、登出、點數錢包的入口嗎？（導覽列） | 分享頁與我的送禮頁（GuestTop）沒有帳號選單、登出、錢包／儲值入口；捐助者只有在 /dashboard、/me/wallet 等創建者頁才看得到 UserMenu。從分享頁要到錢包只能靠眾籌 sheet 的「查看我的點數」，要登出只能手動輸入 /settings；/s 頁頂部導覽文字：WISHLY 探索 靈感 品牌館 我的送禮 繁體中文 English Tiếng Việt Bahasa Indonesia ☀；分享頁 登出/錢包連結/帳號選單 = 0/0/0；我的送禮頁 = 0/0/0 | `D-09.png` |
| D-10 | ✅ | 開認捐 sheet：身分列、數量步進上下限、留言長度、可見度預設、匿名、Esc 放棄確認 | 可見度預設值：private；留言欄 maxlength=200，貼 250 字只保留 200；Esc 時跳出 confirm：「放棄已輸入的內容？」，取消後 sheet 仍在，輸入保留 | `D-10.png` |
| D-11 | ✅ | 認捐 2 件：留言、匿名、可見度「僅好友」→ 成功畫面與期限提示 | 期限提示：請在 2026/11/06 00:33 前到「我的送禮」標記已購買，逾期會自動釋出給其他朋友；DB claims：2\|寶寶出生快樂，阿姨的心意\|t\|friends\|11-05 | `D-11.png` |
| D-12 | ✅ | 另一個未登入訪客停在原頁不重新整理：進度自動更新、顯示「匿名朋友」（SSE 即時更新） | 頁首完成度以「數量」加權：2 件 / 全部 11 件 = 18%，「0 / 5 項」是已全數認領的品項數；觀察者未重整即同步；匿名認捐在公開頁只顯示「匿名朋友」，不洩漏帳號名；連線標示：即時更新中 檢舉此清單 ・服務條款・隱私權政策 | `D-12.png` |
| D-13 | ✅ | 重新整理後仍是「修改我要送的」（身分靠 session cookie） |  | `D-13.png` |
| D-14 | ✅ | 修改認捐：把 2 改成上限 3（可調到剩餘+自己的量），再改回 2；留言一併修改 | 修改畫面沒有可見度／匿名選項（只能在 /me/claims 改可見度；匿名無法再改） | `D-14.png` |
| D-15 | ✅ | 冪等（UI）：快速連點「確定要送」只建立一筆 | POST /claims 實際送出 1 次；DB(筆數\|總數量)=1\|1 | `D-15.png` |
| D-16 | ✅ | 冪等（API 重放）：同 key 同內容回放、同 key 不同內容衝突、新 key 重複認捐被擋 | 同 key 同 body：201 replayed=true；同 key 不同 body：409 code=IDEMPOTENCY_CONFLICT；新 key 對已認捐品項再送：409 code=CLAIM_ALREADY_EXISTS；缺 Idempotency-Key：400 code=IDEMPOTENCY_KEY_REQUIRED | `D-16.png` |
| D-17 | ✅ | 登入逾時：認捐送出時 session 已失效 → 導到登入頁（帶 redirect）→ 重新登入回原清單並認捐成功 | 平台外：直接刪除該帳號所有 sessions 模擬「登入逾時／在別處登出」；導向：http://localhost:3000/login?redirect=/s/VCiD76YAxL | `D-17.png` |
| D-18 | ✅ | 驚喜清單認捐：會員正常認捐；公開頁顯示進度但不顯示捐助者名字 | 捐助者自己看：圖 No.01 50% 生日蛋糕優先度 中 已認領 1 / 2・你要送 1 件 修改我要送的；未登入訪客看：圖 No.01 50% 生日蛋糕優先度 中 已認領 1 / 2 登入後來送；公開 API：claimers_visible=false、item.claimers=null | — |
| D-19 | ✅ | 認捐 API 輸入驗證：數量 0／負數／小數／超大、留言過長、不合法可見度；私人清單的品項 id 被猜到 | qty=0 → 422 VALIDATION_FAILED；qty=-1 → 422 VALIDATION_FAILED；qty=1.5 → 422 VALIDATION_FAILED；qty="a" → 422 VALIDATION_FAILED；qty=10000 → 422 VALIDATION_FAILED；note 201 字 → 422 VALIDATION_FAILED；visibility=bogus → 422 VALIDATION_FAILED；對私人清單品項（猜到 id）認捐：404 NOT_FOUND；對被下架清單品項認捐：410 WISHLIST_REMOVED；對不存在的品項認捐：404 | `D-19.png` |
| D-19b | ✅ | 惡意顯示名稱／留言（HTML）：公開頁看到的是純文字不執行；同時對照「預設可見度＝私人」與「選公開」的顯示差異 | DB 兩筆認捐的可見度：private,public；公開頁（show_claimer_names=開）奶粉卡：圖 No.04 67% 奶粉優先度 中 已認領 2 / 3 匿名朋友、<img src=x onerror="window.__xss=1"> 要送 登入後來送 | — |
| D-20 | ✅ | 同事小陳（UI 登入）與小林（同時）搶「嬰兒帽」最後 1 件：只有一人成功 | 小陳成功=true、小林成功=false；DB(筆數\|總數量)=1\|1、qty_claimed=1 | `D-20.png` |
| D-21 | ⚠️ | 搶輸的人：看到什麼訊息、多久後 sheet 自動關閉、關閉後頁面還留不留提示；贏家看到成功 | 搶輸時訊息只在 sheet 內閃現約 1.2 秒就隨 sheet 關閉，頁面沒有留下任何 toast 或說明，使用者可能只看到卡片突然變「已全數認領」；輸家 sheet 內橫幅：lose:剛剛被別人認領完了。；輸家 sheet 已自動關閉（ClaimSheet.vue 以 1200ms 計時關閉；腳本偵測時已關）；sheet 關閉後頁面上殘留的提示文字數：0 | — |
| D-22 | ✅ | 部分搶購：「尿布」剩 3，兩人各要 2 → 輸家被告知「只剩 1 個」、上限調為 1，改送 1 成功 | 輸家訊息：抱歉，只剩 1 個了。 | `D-22.png` |
| D-23 | ✅ | API 層併發：6 個新會員同時搶 1 件 → 恰 1 個 201、其餘 409，qty_claimed=1 | 回應：409ITEM_FULLY_CLAIMED、409ITEM_FULLY_CLAIMED、409ITEM_FULLY_CLAIMED、409ITEM_FULLY_CLAIMED、201、409ITEM_FULLY_CLAIMED；DB(筆數\|總數量)=1\|1、qty_claimed=1 | — |
| D-23b | ✅ | 同一人開兩個分頁同時送（不同 Idempotency-Key）：只成立一筆，另一筆被擋 | 回應：201、409CLAIM_ALREADY_EXISTS、409CLAIM_ALREADY_EXISTS；DB(筆數\|總數量)=1\|1、qty_claimed=1 | — |
| D-24 | ✅ | /me/claims：分組、狀態徽章、期限、可見度選單；改可見度後重新整理仍保留 | 可見度 friends → private，重整後保留；/me/claims 頂端身分卡：阿姨ec8（已登入） | `D-24.png` |
| D-25 | ✅ | 延長購買期限 7 天（一次為限） | 期限：請在 2026/11/06 00:33 前標記已購買 → 請在 2026/11/13 00:33 前標記已購買 | `D-25.png` |
| D-26 | ✅ | 標記已購買 → 已送達（揹巾）：徽章更新、期限與延長鈕消失、已送達後不能再取消 | 已送達後沒有任何動作鈕 | `D-26.png` |
| D-27 | ✅ | 取消認捐（confirm 對話框）→ 移到「過去的紀錄」，數量釋出回分享頁 | confirm 文字：「確定不送這份禮物了嗎？」；觀察者（未重整）也同步看到 0 / 3 | `D-27.png` |
| D-27b | ✅ | 取消後再認捐同一品項（已取消的紀錄不擋重新認捐） | DB 該使用者對奶瓶組的紀錄：cancelled:2,reserved:1 | `D-27b.png` |
| D-28 | ✅ | 購買期限逾期：把「圍兜」認捐期限改成過去（時間快轉），等背景工作釋放；先確認逾期前狀態 | 平台外：UPDATE claims SET expires_at = now()-1min（claim 01a18882-f31d-7387-991c-65db4b5a95fa）；背景 expire_due 每 5 分鐘跑一次，結果於 D-29 檢查（本腳本尾端）；改時間後、tick 前 qty_claimed=1（仍佔用） | — |
| D-29 | ✅ | 購買期限逾期釋放（接 D-28）：背景工作跑完後認捐變「已逾期」、數量釋出、收到逾期信 | 等了 116 秒；claim 狀態=expired；qty_claimed=0；阿姨信件：【Wishly】「圍兜」已超過購買期限｜【Wishly】《ec886c47 已結束清單》已結束，請標記你要送的禮物｜Wishly 註冊驗證碼 | `D-29.png` |
| D-30 | ✅ | 已結束清單：先有認捐 → 受捐者結束清單 → 分享頁橫幅、無送禮鈕；我的送禮仍可標記已購買但不可延長 | 平台外：先以 API 為阿姨建一筆認捐，再由受捐者(API)結束清單；對已結束清單直接打 API：409 WISHLIST_CLOSED | `D-30.png` |
| D-31 | ✅ | 私人清單與不存在的 slug：404 頁（頁面與 HTTP 狀態） | HTTP 狀態（私人 / 不存在）：404 / 404；私人清單對非擁有者與「不存在」回應相同（不洩漏存在） | `D-31.png` |
| D-32 | ✅ | 被下架清單：410 頁、不洩漏標題、HTTP 410 | HTTP 狀態：410 | `D-32.png` |
| D-33 | ✅ | 好友限定清單・未登入：看到「請先登入」，登入後（非好友）回到原頁看到「沒有權限」與加好友鈕 | 403 頁顯示擁有者名稱：這份清單只開放給擁有者的好友（擁有者：小愛ec8）；SSR HTTP 狀態：200 | — |
| D-34 | ✅ | 送出好友邀請 → 受捐者接受（API）→ 重新整理後看得到限定清單並能開認捐 sheet | 受捐者端 /friends 回應鍵：friends,incoming,outgoing；平台外：受捐者以 API 接受邀請；好友限定清單的認捐成功（受限路徑也可認捐） | — |
| D-35 | ✅ | 好友邀請連結 /invite/[token]：未登入→登入→成為好友→跳到該清單 | 邀請回應鍵：expires_at,token,url,wishlist；網址：http://localhost:3000/invite/5WXnsK8Z2ONfk3YXS8PzHZ7pq8BN1NzHoUlZq8Ai7nM | — |
| D-36 | ✅ | 失效／亂填的邀請連結：顯示「已失效或被撤銷」 |  | — |
| D-37 | ✅ | 檢舉清單：訪客（免登入）選原因送出 → 「已收到檢舉」 | DB content_reports：scam\|open；本環境 Turnstile 未設 site key，前端送 dev-bypass；真實人機驗證見 D-91 | `D-37.png` |
| D-38 | ✅ | 離線／斷線：顯示離線橫幅、送禮鈕停用，恢復後可用 | 離線時第一個送禮鈕 disabled=true | `D-38.png` |
| D-39 | ✅ | 切換語系（English）：分享頁文案改變並保留登入 | 切換後頁面：Img No.01 33% 奶瓶組Priority: Medium Claimed 1 / 3 · you'll give 1 Anonymous will give this Edit my gift | `D-39.png` |
| D-40 | ✅ | /explore：列出公開且有品項的清單，不含好友／私人／草稿／空清單／下架；點卡片進分享頁 | 出現次數：{"main":1,"surprise":0,"friends":0,"private":0,"closed":0,"gone":0,"draft":0,"empty":0}；（若清單多於一頁會按「載入更多」）最終 main=1；驚喜清單 不會出現在探索頁；已結束清單 不會出現 | `D-40.png` |
| D-41 | ✅ | /inspire 選物靈感：篩選器、空狀態或商品卡 | 目前 catalog 商品數（API）：13；有商品，已試篩選 | — |
| D-42 | ✅ | /brands 品牌館與品牌頁（營運先種一個品牌與商品作前置） | 前置：營運建品牌 201 ；前置：營運建商品 201 ；品牌頁商品：INDEX / 031 BRAND FILE Q QA 品牌 ec886c47 捐助者流程測試品牌 Q 01 QA 品牌 EC886C47 QA 商品 ec886c47 NT$1,280 加到我的願望清單 誰想要這個 看商品 | — |
| D-43 | ✅ | 首頁（未登入）：主要入口可達 /explore；頁首導覽四個連結 |  | — |
| D-50 | ✅ | 餘額 0 的人按「心願眾籌」：看到餘額、快速金額、不足提示；送出被擋並引導「查看我的點數」 | 送出後訊息：點數不足（目前餘額 0 點） 查看我的點數 | `D-50.png` |
| D-51 | ✅ | /me/wallet（新帳號）：餘額 0、無贊助、無可退現金說明、儲值入口 | 頂部：‹ WISHLY 繁體中文 English Tiếng Việt Bahasa Indonesia ☀ 阿 阿明ec8 ▾ | `D-51.png` |
| D-52 | ✅ | /me/topup 前端驗證：金額低於 100、超過 50,000、非整數 | 金額 50 → 金額須介於 100 至 50,000；金額 60000 → 金額須介於 100 至 50,000；金額 12.5 → 請輸入整數金額 | `D-52.png` |
| D-53 | ✅ | 儲值 NT$1,000（信用卡）：建單 → 以隱藏表單 POST 導向綠界（攔截），DB 有待付款單 | 導向 https://payment-stage.ecpay.com.tw/Cashier/AioCheckOut/V5；欄位：CheckMacValue,ChoosePayment,ClientBackURL,EncryptType,ItemName,MerchantID,MerchantTradeDate,MerchantTradeNo,PaymentInfoURL,PaymentType,ReturnURL,TotalAmount,TradeDesc；TotalAmount=1000；ClientBackURL=http://localhost:3000/me/topup；OrderResultURL=(無) | `D-53.png` |
| D-54 | ✅ | 付款完成（綠界通知，替身）→ 回到 /me/topup 與錢包：已入帳、餘額 1,000 | 平台外：ecpayPaid(Tf4c6f05927ac4e529d4) → 200 1\|OK；錢包：可退現金 可退現金 1,000 點；信件：Wishly 註冊驗證碼 | `D-54.png` |
| D-55 | ✅ | 儲值選超商代碼 → 取號中（待綠界 /info 通知）→ 取消待付款訂單 | 待付款單文字：NT$ 500 超商代碼 待付款 建立 2026/10/30 00:35 取號中…（約需數秒） 取消訂單；取消後該單狀態：failed | `D-55.png` |
| D-56 | ✅ | 贊助 100 點（留言、可見度）→ 成功畫面、卡片進度與贊助者名單、錢包餘額更新 |  | `D-56.png` |
| D-57 | ✅ | 贊助超過剩餘：他人先贊助 150 後，自己開著的 sheet 即時更新剩餘；超額被原生驗證擋、API 超額回 CROWDFUND_TARGET_EXCEEDED；改送 50 達標 | 超額 250 點：送出被瀏覽器原生驗證擋下（max=剩餘 200），沒有頁內 alert，原生提示「Value must be less than or equal to 200.」；他人贊助後，已開啟的 sheet 在未重開下即時改為「還差 50 點達標」；改填 100：原生驗證訊息 Value must be less than or equal to 50.；API 直接超額贊助 100：409 code=CROWDFUND_TARGET_EXCEEDED remaining=50；達標橫幅：有 | `D-57.png` |
| D-58 | ✅ | /me/wallet：我的贊助列表、狀態徽章、帳本；撤回搖籃的贊助 | 前置：以 API 為阿明贊助搖籃 80 點（UI 贊助流程已在 D-56 驗證）；贊助列：無圖 搖籃 ec886c47 眾籌清單 80 點 已贊助募集中 「QA」 贊助 2026/10/30 00:35 誰看得到 公開 僅好友 私人 撤回 ｜ 無圖 嬰兒推車 ec886c47 眾籌清單 50 點 已達標扣用已達標 贊助 2026/10/30 00:35・達標 2026/10/30 00:35 誰看得到 公開 僅好友 私人 ｜ 無圖 嬰兒推車 ec886c47 眾籌清單 100 點 已達標扣用已達標 「推車我出一點」 贊助 2026/10/30 00:35・達標 2026/10/30 00:35 誰看得到；帳本： 00:35 誰看得到 公開 僅好友 私人 無圖 嬰兒推車 ec886c47 眾籌清單 100 點 已達標扣用已達標 「推車我出一點」 贊助 2026/10/30 00:35・達標 2026/10/30 00:35 誰看得到 公開 僅好友 私人 點數帳本 贊助 2026/10/30 00:35・餘額 770 -80 贊助 2026/10/30 00:35・餘額 850 -50 贊助 2026/10/30 00:35・餘額 900 -100 topup 2026/10/30 00:35・餘額 1,000 +1,000 | `D-58.png` |
| D-59 | ✅ | 眾籌截止後轉投：玩具車贊助 70 點 → （快轉：標記截止）→ 錢包「轉投其他品項」→ 搖籃 | 平台外：以 SQL 模擬 tick_funding 把玩具車標為 expired（背景工作每 5 分鐘一次）；贊助 70 點以 API 建立；轉投後各列：無圖 搖籃 ec886c47 眾籌清單 70 點 已贊助募集中 贊助 2026/10/30 00:35 誰看得到 公開 僅好友 私人 撤回 ｜ 無圖 玩具車 ec886c47 眾籌清單 70 點 已轉投已截止 贊助 2026/10/30 00:35 誰看得到 公開 僅好友 私人 ｜ 無圖 搖籃 ec886c47 眾籌清單 80 點 已退回募集中 「QA」 贊助 2026/10/30 00:35 已退回差額 80 點 誰看 ｜ 無圖 嬰兒推車 ec886c47 眾籌清單 50 點 已達標扣用已達標 贊助 2026/10/30 00:35・達標 2026/10/30  ｜ 無圖 嬰兒推車 ec886c47 眾籌清單 100 點 已達標扣用已達標 「推車我出一點」 贊助 2026/10/30 00:35・達標 2；DB：嬰兒推車:100:captured, 嬰兒推車:50:captured, 搖籃:80:released, 玩具車:70:reallocated, 搖籃:70:pledged | `D-59.png` |
| D-60 | ✅ | 修改贊助可見度（錢包頁） |  | `D-60.png` |
| D-61 | ✅ | /me/refund 現金退款：輸入驗證（0、小數、超額）→「全部」→ 送出 200 → 餘額先扣、列為待審核 | 錢包餘額 780；可退現金 780；點數 0 → 點數須介於 1 至 50,000；點數 12.5 → 請輸入整數點數；點數 99999999 → 點數須介於 1 至 50,000；「全部」填入：780；退款後餘額 580（先扣 200）；DB cash_refund_requests：requested\|200 | `D-61.png` |
| D-62 | ✅ | 手機 390 寬的錢包／儲值／退款頁：無橫向捲動、主要按鈕可點 | /me/wallet：頁面寬度 OK；標題 我的點數；/me/topup：頁面寬度 OK；標題 儲值；/me/refund：頁面寬度 OK；標題 申請現金退款；/me/claims：頁面寬度 OK；標題 我的送禮；「前往付款」高度 44px | `D-62.png` |
| D-63 | ✅ | 贊助冪等：同一把 Idempotency-Key 重送兩次，錢包只扣一次 | 第一次 201、第二次 201 replayed=true；餘額 580 → 560；缺 Idempotency-Key：400 IDEMPOTENCY_KEY_REQUIRED | — |
| D-64 | ✅ | 現金退款第二次：超過剩餘可退現金額被擋、不會變成負數 | 可退現金 560；輸入 561 → 不可超過可退現金餘額；API 直接超額申請：409 {"balance":560,"code":"INSUFFICIENT_POINTS","detail":"點數餘額不足（目前 560 點）","status":409,"title":"INSUFFICIENT_POINTS","type":"https://wishly.tw/problems/insufficie；申請列表：數 全部 不可超過可退現金餘額 確認內容 輸入點數後顯示摘要。 申請後點數會先扣除；審核通過後退款。 若申請被拒絕，點數會退回你的錢包。 送出申請 我的申請 200 點 待審核 申請 2026/10/30 00:35 審核中，通過後會退款。 | `D-64.png` |
| D-70 | ✅ | 救災：/e → 事件頁 → 據點分享頁（組織資訊、地址）→ 認捐瓶裝水成功 | 平台外：SQL 種子組織/許可/事件/兩個據點（oae33c6a0d/q910ec6fe4/w5ec719546）；據點地址連結數（頁面 DOM）：2；可見數：2；此流程中的 4xx 回應： | `D-70.png` |
| D-71 | ✅ | 救災・組織頁 /o/[slug]、暫停收件的據點（橫幅、送禮鈕停用） | 平台外：SQL 把第二個據點改為 paused（模擬組織暫停）；暫停據點送禮鈕 disabled=true；直接打 API 認捐暫停據點：409 SITE_PAUSED | `D-71.png` |
| D-72 | ⚠️ | 退訂連結 /unsubscribe：二步確認（開頁不退訂）→ 確定退訂 → 已退訂；亂填 token 顯示失效 | 退訂的是「生日通知」(kind b)，但頁面標題寫「確定要退訂送禮通知？」、完成後寫「之後不會再寄『有人來送』的通知信」，文案與實際效果不符；GET /api/v1/unsubscribe → 302 Location=http://localhost:3000/unsubscribe?token=YjowMWExODg4My1mZTM4LTcxZWItYTUyOS1hMWY1ZTE3ZjEzMWM6MTc5MzM3ODE3Mw.335ac9a015aef5028665e5d84e11278cad34d76a6e98c3657c0bbc03c43a34f7；平台外：以開發機密 dev-unsub-secret 簽出「生日通知(b)」退訂 token（真實信件由 notify.rs 產生）；notification_prefs {"email_claims": true} → {"email_claims": true, "email_birthdays": false}；頁面文案：已退訂 之後不會再寄「有人來送」的通知信。想重新開啟，登入後到「帳號設定」即可。 前往帳號設定 | — |
| D-73 | ⚠️ | 未登入直接進會員頁：/me/claims（空狀態有無登入引導）、/me/wallet /me/topup /me/refund（導登入） | /me/claims 未登入只顯示「你還沒有要送的禮物…登入會員就能認領」空狀態，沒有任何「登入」連結；而 /me/wallet 等會直接導去登入，行為不一致；/me/claims 未登入內文：WISHLY / CLAIMS 我的送禮 你還沒有要送的禮物。收到朋友的清單連結後，登入會員就能認領，並在這裡管理。 看不到之前以訪客身分認領的禮物？可能是在不同瀏覽器或 LINE 內建瀏覽器。留過 Email 的，可從確認信的「管理我的送禮」連結回來。；/me/wallet → /login?redirect=/me/wallet；/me/topup → /login?redirect=/me/topup；/me/refund → /login?redirect=/me/refund | — |
| D-74 | ✅ | 信件盤點：阿姨（認捐、取消、延長…）實際收到哪些信 | 阿姨收到：【Wishly】《ec886c47 已結束清單》已結束，請標記你要送的禮物｜Wishly 註冊驗證碼；受捐者小愛收到：Wishly 註冊驗證碼；阿姨的排程通知（購買提醒等）：claim.list_closed/sent/10-29 16:35；claim.purchase_reminder/pending/11-02 16:33；claim.purchase_reminder/pending/11-02 16:34；claim.purchase_reminder/pending/11-02 16:34；claim.purchase_reminder/pending/11-02 16:34；claim.purchase_reminder/pending/11-02 16:34；claim.purchase_reminder/pending/11-02 16:34；claim.purchase_reminder/pending/11-04 16:33；claim.purchase_reminder/pending/11-04 16:34；claim.purchase_reminder/pending/11-04 16:34；claim.purchase_reminder/pending/11-04 16:34；claim.purchase_reminder/pending/11-04 16:34；claim.purchase_reminder/pending/11-04 16:34；claim.purchase_reminder/pending/11-09 16:33；c… | — |
| D-90 | ⛔ | 真機 LINE 內建瀏覽器（cookie 保留、「用預設瀏覽器開啟」、分享預覽圖） | 需真機與 LINE app；本環境只用 UA 字串模擬 isLineBrowser（D-02）。分享預覽（OG 圖）只能驗 SSR meta（D-01），LINE 爬蟲實際抓圖不可驗 | — |
| D-91 | ⛔ | 第三方登入（Google / LINE）、Turnstile 真實驗證 | 需真實 OAuth 供應商與 Cloudflare Turnstile site key；登入頁按鈕只確認存在；檢舉 / 註冊在無 site key 時送 dev-bypass | — |
| D-92 | ⛔ | 真實綠界付款頁、超商代碼／ATM 取號、發票 | 綠界 stage 需外網；只驗到「導向表單欄位正確」與「簽章通知入帳」（替身）。超商代碼／ATM 的取號資訊需 /info 通知，替身未實作 | — |
| D-93 | ⛔ | CDN 快取（/s/slug 公開回應快取、受限清單 no-store、purge） | 本地沒有 Cloudflare；僅在 D-33/D-34 驗證受限清單走帶憑證路徑 | — |
| D-94 | ⛔ | 原生殼（Tauri）App 內儲值導向網頁版 | 需 Tauri 殼；topup.vue 有 native 分支但此環境為純網頁 | — |

## 6. 問題清單

本次沒有 ❌（無功能性 bug）；以下全是 ⚠️ 缺口、設計落差或文件不一致。嚴重度：中＝營運或使用者實際會卡住／蒙在鼓裡；低＝體驗、一致性。

| 編號 | 嚴重度 | 類型 | 重現與預期／實際 | 相關程式碼 | 建議修法 | 建議 |
|---|---|---|---|---|---|---|
| P-P1 | 中 | 設計缺口 | P-69：全站 7 個頁面、API 8 個候選端點都沒有聯絡客服的管道；但下架、停權、錢包凍結、帳號 ID 等處的文案都寫「請聯絡客服」。預期有信箱／表單；實際沒有，後台也沒有接收客訴或工單的地方。 | 文案：`apps/web/i18n/locales/zh-Hant/lists.json:43`、`me.json:175`、`guest.json:231`、`auth.json:7`、`settings.json:34` | 先在頁尾與錯誤文案放客服信箱／LINE 官方帳號；再評估簡易 `support_requests` 表（帶 user_id、類型、關聯對象）加後台佇列 | 開 issue |
| P-P2 | 中 | 設計缺口 | P-22／P-23：事件下架 → 掛在其下的上線據點自動 `paused`，組織 owner 沒收到任何通知；取消下架後據點仍 `paused`（P-24），營運沒有恢復入口，只能靠組織自己發現。 | `apps/api/src/relief_events.rs:316`（moderate 只寫 audit／purge CDN，無 `notifications`） | 下架暫停據點時比照 `org.permit_revoked` 對 owner 寄信（含原因）；取消下架時一併通知；文件註明據點須組織自行恢復 | 直接修（通知）＋ 文件 |
| P-P3 | 中 | 設計缺口 | P-16：許可被退件（含備註）時申請人沒收到信，只能自己回狀態頁看；只有「撤銷已核准許可」會寄。 | `apps/api/src/org_admin.rs:198`（review_permit，notifications 只在 `status == "approved"` 分支） | pending 退件也寫 `org.permit_rejected` 通知並帶備註 | 直接修 |
| P-P4 | 中 | 設計缺口 | P-58／P-63／P-66：現金退款完成或被拒絕、錢包凍結（爭議）、清單下架／恢復、帳號停權，使用者都沒有信。使用者只有在動作失敗時才看到「請聯絡客服」。 | `apps/api/src/wallet_ops.rs:199`（cash_refund_patch）、`apps/api/src/reports.rs:275`（moderate）、`apps/api/src/admin.rs` update_user、`wallet_ops.rs` dispute_open | 為這幾個事件各加一種 `notifications.kind` 並在 `notify.rs` 補文案 | 開 issue（需決定哪些要主動通知） |
| P-P5 | 低～中 | 設計缺口 | P-48：採購單已出貨／已送達／失敗只通知捐助者，受捐者（清單擁有者）一封信都沒有，只能自己回進度頁。 | `apps/api/src/orders.rs:274`、`:284`、`:302`（只呼叫 `enqueue_donors`） | 另對 `wishlists.owner_id` 排一封（驚喜模式要略過或改寫文案，避免洩漏） | 開 issue（牽涉驚喜模式） |
| P-P6 | 中 | 設計缺口 | P-31／P-32：撤銷組織 = `rejected` 且鎖 30 天，據點被降為 draft；後台對已退件組織沒有任何動作（卡片只有「收合」）。誤撤銷只能 SQL 補救。 | `apps/api/src/org_admin.rs:93`（review：reject 只允許 pending／verified，無 rejected → verified）；`apps/web/app/pages/admin/orgs.vue:54` | 新增「恢復驗證」動作（需原因、寫稽核、不自動恢復據點），或撤銷時可選「不鎖 30 天」 | 開 issue |
| P-P7 | 中 | 設計缺口 | P-80／P-81：首頁 KPI 只有使用者／清單／使用中／已下架／待處理檢舉。待審組織（6）、待下單採購單（11）、待審現金退款（7）、爭議中儲值、未結欠款都要逐頁點進去才知道，沒有計數或紅點。 | `apps/api/src/admin.rs:527`（stats_json）、`apps/web/app/pages/admin/index.vue`（nav 磚） | stats 加 `pending_orgs`、`pending_orders`、`requested_refunds`、`disputed_topups`、`open_debts`，nav 磚上顯示數字 | 直接修 |
| P-P8 | 中 | 功能缺口 | P-74：沒有公告、系統維護通知、群發信或站內訊息；事故時只能開唯讀旗標，無法告知使用者；品牌活動也無處發布。`notifications` 只有交易型 kind。 | 無（`apps/api/src/notify.rs` 僅有業務事件） | 最小可行：`system_notices`（標題、內文、起訖時間、等級）＋ 公開 `/public/status` 帶出 ＋ 前端頂部橫幅；群發信另案（需退訂與寄信量控管） | 開 issue |
| P-P9 | 低 | 設計落差 | P-41／P-85：`GET /admin/orders` 每回傳一張單就寫一筆 `order.view_address`，本次送禮執行人 141 筆、金流 25 筆（同一批單重複載入／換篩選都再寫）。稽核被噪音淹沒，也分不出「有意查看」與「列表帶出」。 | `apps/api/src/orders.rs:112-124` | 列表預設只回遮罩地址，點「顯示收件資訊」才解密並寫稽核；或同 staff／同單 N 分鐘內去重 | 開 issue |
| P-P10 | 低 | 不一致 | P-85：staff 下載組織立案文件的稽核 `actor_type` 是 `user`（用 `audit_user`），其他營運動作都是 `staff`。依 `actor_type=staff` 統計會漏掉最敏感的文件讀取。 | `apps/api/src/orgs.rs:522`（download）、`:80`（audit_user） | download 內 staff 分支改呼叫 `admin::audit` | 直接修 |
| P-P11 | 低 | UI | P-22：事件下架原因後端必填，但輸入欄只標「原因」、按鈕沒有 disabled；送出才得到「下架須填寫原因」。清單下架（`index.vue`）則有必填標示並 disabled。 | `apps/web/app/pages/admin/relief-events.vue:74`、`apps/web/i18n/locales/zh-Hant/admin.json`（`events.reasonLabel`） | 下架時標「（必填）」並 `:disabled="kind==='hide' && !reason.trim()"` | 直接修 |
| P-P12 | 低 | UI | P-64：結案檢舉（已處理／駁回）沒有備註欄，後端 `note` 欄位閒置；檢舉卡片只看得到檢舉者類型，看不到同清單累計與歷史（下架後清單列顯示「被檢舉 0 次」，因為只算未結案）。 | `apps/web/app/pages/admin/index.vue`（`handle()` 只送 status）、`apps/api/src/reports.rs:217`（Handle.note）、`apps/api/src/admin.rs:139`（open_report_count） | 結案時帶備註；清單列同時顯示累計檢舉數 | 直接修 |
| P-P13 | 低 | UI | P-31：撤銷組織第二段確認沒有備註欄（理由只能在第一段填）；退件／撤銷的備註後端不強制，申請人可能收到「審核備註：-」的信。 | `apps/web/app/pages/admin/orgs.vue:144`、`apps/api/src/org_admin.rs:93` | 退件與撤銷要求備註（至少 5 字），第二段顯示第一段備註 | 直接修 |
| P-P14 | 低 | 功能缺口 | P-51：儲值查詢只有狀態篩選，沒有 Email／交易序號搜尋；為了找一筆已入帳單要載入 15 張卡片。列上顯示完整 Email，與「使用者」頁預設遮罩不一致。 | `apps/web/app/pages/admin/topups.vue:17`、`apps/api/src/wallet_ops.rs:466`（只有 status 與 cursor） | `q` 參數支援 Email／`merchant_trade_no`／provider_txn_id；Email 預設遮罩 | 開 issue |
| P-P15 | 低 | 文件／文案 | P-01：非 staff 打開 `/admin` 顯示「403」大字，但標題與說明是「找不到頁面／這個網址不存在」。程式註解標明是刻意（不暴露入口），但狀態碼與文案互相矛盾。 | `apps/web/app/middleware/staff.ts`（abortNavigation 403） | 擇一：顯示真的 404，或 403 配「沒有權限」文案；handbook 6 章同步 | 直接修 |
| P-P16 | 低 | 功能缺口 | P-82／P-83：分析頁只有使用者成長、清單、送禮、檢舉／登入失敗與分布；沒有儲值金額、點數發放／消耗、退款／拒付、採購週期（下單→送達）、組織審核等待時間。金流對帳要靠 SQL。 | `apps/api/src/admin.rs:553`（analytics） | 另開「營運／金流」分頁，指標來自 `topups`、`point_ledger`、`purchase_orders`、`organizations` | 開 issue |
| P-P17 | 低 | UI | P-11／P-14：組織詳情裡 `org_type` 顯示原始英文（`foundation`）；文件列只有「勸募許可文件・日期」，多份時看不出哪份屬於哪張許可（許可列也沒有附檔連結）。 | `apps/web/app/pages/admin/orgs.vue`（dl 輸出 `detail.org[f]`、documents 迴圈） | 類型用 i18n 標籤；許可列旁放其 `document_id` 的下載鈕 | 直接修 |
| P-P18 | 低～中 | 設計落差 | P-91：MFA 只是「建議」。`cs` 重設 `lead` 的 MFA 後，`lead` 重新登入立刻可用全部後台功能（含發點、撤銷、停權），並只顯示建議提示；任何一位 staff 都能重設另一位的 MFA，且不需第二人核可。 | `apps/api/src/admin.rs:35`（`require_staff` 的 ponytail 註解）、`admin.rs:360`（reset_mfa） | 對高風險動作（發點、停權、staff 授權、撤銷）強制 MFA；重設 MFA 要求第二位 staff 核可或冷卻時間 | 開 issue |

## 7. 營運角色目前缺少但應有的能力

| 能力 | 現況（本次驗證） |
|---|---|
| 客服工單／客訴通道、用戶回饋入口 | 沒有（P-P1）。後台無工單、無案號、無處理時效；停權／下架申訴沒有流程 |
| 對外公告／維護通知／群發 | 沒有（P-P8） |
| 待辦總覽與工作佇列計數、指派與認領 | 沒有（P-P7）；採購單有承辦人欄位但沒有「認領」，多人同時處理靠衝突訊息（P-47 驗證有效） |
| 批次操作（多張採購單出貨、批次結案檢舉、批次核准許可） | 沒有，全是逐筆確認 |
| 儲值／使用者／採購單的跨欄位搜尋 | 只有 `/admin/wallets`、使用者、清單可搜；儲值、採購單、退款、欠款、組織只能用狀態篩選（P-P14） |
| 處理結果通知使用者（退款、凍結、停權、下架、據點暫停、許可退件） | 大多沒有（P-P2、P-P3、P-P4） |
| 還原誤操作（誤撤銷組織、誤合併事件、已完成退款） | 組織撤銷（P-P6）、事件合併（P-25 驗證不可逆）、退款完成皆無後台復原路徑 |
| 組織據點的營運恢復／代為暫停單一據點 | 沒有單一據點操作；只能整個事件、整張許可或整個組織 |
| 金流營運報表（對帳、拒付率、退款待處理金額） | 沒有（P-P16）；對帳不變式只能 SQL（P-5A） |
| 檢舉者回覆、重複檢舉合併、檢舉人黑名單 | 沒有；匿名檢舉無聯絡方式 |
| 營運動作的匯出 | 稽核頁無匯出（只能逐頁載入） |

## 8. 無法驗證或未執行（⛔／未涵蓋）

| 項目 | 原因 |
|---|---|
| P-92 全站唯讀模式 | ✅（2026-10-29 四個角色跑完後由主審以 `FLOW_ONLY=J FLOW_READONLY=1 node qa/role-flows/operator.mjs` 單獨執行）客服在 `/admin` →「系統旗標」開啟並確認 → 一般用戶建立清單回 503 `READ_ONLY_MODE` → 後台旗標仍可關閉；關閉後 `/public/status` 為 `read_only: false`。原本因會干擾平行執行的其他流程而延後 |
| 最後一位 staff 保護（`LAST_STAFF`） | 共用環境有其他 agent 的 staff，無法觸發；僅讀碼確認（`admin.rs` protect_last_staff） |
| 真實綠界付款／退款／拒付 webhook | 付款通知用替身模擬；現金退款「原路退回」是營運人員到金流商後台人工作業，系統不呼叫；供應商爭議 webhook 目前只有 501 接口 |
| Cloudflare CDN 快取清除 | 本環境沒有 CDN，只驗證源站狀態碼；`cdn::purge_*` 呼叫未驗證 |
| 組織許可到期自動暫停（`permit_expiry_tick`，每小時一次） | 未執行排程；需時間快轉 |
| LINE／Google 帳號的復原 | 沒有 Email 的帳號後台按鈕 disabled，無核驗流程可測 |
| 事件管理列表的已隱藏事件 | 列表來源是公開 API（不含已隱藏），重新整理後隱藏的事件只能用 UUID 操作（`relief-events.vue` 的 `direct` 分支）；本次只在同一次載入內取消隱藏，未驗證重新整理後的路徑 |
| Logo 上傳的 R2 條件式 PUT 語意 | S3 替身為 moto，不保證相同語意；本次上傳成功（`logo_status=ready`），僅供參考 |
| 非 zh-Hant 語系的營運介面與信件 | 未涵蓋 |

## 9. 附註

- 本次產生的資料（組織、事件、採購單、儲值等）都帶標籤 `T`，只操作自己建立的資料；為了讓腳本可重跑，檢舉限流計數在 P-60 清除，OTP 限流由 `registerApi` 清除，皆為平台外動作。
- P-14 的「文件下載鈕 4 個」包含補件新增的立案文件與許可文件，不是「立案文件 4 份」。
- 對帳不變式（P-5A）於全庫檢查（含其他 agent 資料）：`balance ≠ Σ lots` 與 `balance ≠ Σ ledger` 的錢包皆為 0 個。
