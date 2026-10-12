# 登入方式與 Email 政策

- 設定頁可新增或解除 Email、Google、LINE。Google／LINE 在原生殼仍不提供 OAuth 入口，請使用網頁版。
- 手動 OAuth 綁定須保留發起的同一個 session；state 簽章包含供應商、帳號、session、用途及 10 分鐘期限。Email 綁定 OTP 同樣限定帳號、session 與用途，不能拿來登入、註冊或重設密碼。
- 已屬於其他帳號的 provider identity 或已驗證 Email 回 `409 IDENTITY_IN_USE`。不合併既有兩個帳號，也不搬移清單、認領、點數與訂單。
- 至少保留一種登入方式。解除以交易鎖保護；註冊與密碼重設的 identity／密碼修改也在同一交易內核對所有權，避免並行解除被舊請求覆蓋。

## 解除與重新連結

自助解除成功時會在同一交易撤銷其他 session，目前裝置保持登入；解除失敗不撤銷 session。後台解除則撤銷該帳號所有 session。

解除會移除目前的登入方式；不代表封鎖供應商或永久放棄信箱所有權。仍沿用已驗證 Email 自動連結政策：日後重新完成 Email OTP、註冊／密碼重設，或 Google verified-email 登入，可能連結回原帳號，並再次寄送安全通知。未通過驗證不能重建登入方式。

聯絡 Email 只取自仍連結的已驗證 identity。若解除後還有 Google 提供同一 Email，保留聯絡信箱；沒有剩餘來源則清除。Email 登入與通知信箱使用同一地址；更換不同信箱須先解除原本提供聯絡信箱的方式。

## 沒有 Email 的 LINE 帳號

允許使用，但無法收取 Email 通知或用密碼重設找回帳號，也沒有 LINE 通知替代通道。設定頁顯示限制並停用 Email 通知開關。建議在失去 LINE 登入能力前，驗證 Email 或綁定另一種登入方式。Email 綁定後可使用「設定／重設 Email 登入密碼」；重設會登出所有裝置。後台可用名稱、帳號代號或 UUID 搜尋。

新增／解除登入方式的安全通知沿用 durable outbox，不受 `email_claims` 或認領通知退訂影響，寄往變更時的已驗證信箱；解除後即使聯絡 Email 清空，也保留該次安全信的收件人。沒有任何已驗證 Email 時無法寄送安全信；帳號刪除仍會取消待寄通知，並清除安全信收件地址與該帳號所有綁定驗證碼。

## 頭像與 JWKS

OAuth 頭像只接受 HTTPS 的 Google／LINE 圖片來源，拒絕重新導向，限制下載時間與大小（5 MB），沿用圖片重編碼及中繼資料移除後寫入 S3 相容儲存。`avatar_key` 新資料只存 `avatars/<uuid>.<ext>`；舊外部 URL 在下次 OAuth 登入或綁定時惰性轉存，不再直接公開。轉存以 `tokio::spawn` 背景執行，登入或綁定不等待下載及 S3 上傳；程序重啟中斷時於下次登入重試。轉存失敗使用預設頭像，不阻擋登入；不覆蓋已儲存的自家頭像。

Google JWKS 使用單一程序內的記憶體快取，依 `Cache-Control`／`Age` 到期；沒有 max-age 時預設 5 分鐘，最多保留 24 小時。`no-store`、`no-cache`、`max-age=0` 不重用；未知 kid 可觸發一次重新抓取，額外更新至少間隔 30 秒，支援金鑰輪替並限制偽造 kid 的上游流量。更新失敗不接受過期金鑰。多個 API 程序各自快取。

來源：[Google OpenID Connect 驗證及快取指引](https://developers.google.com/identity/openid-connect/openid-connect)、[LINE 使用者個人資料](https://developers.line.biz/en/docs/line-login/managing-users/)。

## 新增 API 與 migration

- `POST /me/identities/email/request`：`{email}`，已登入；202，沿用 OTP 限流，另加每帳號每小時 5 次。
- `POST /me/identities/email/verify`：`{email, code}`，回更新後的 `/me` 資料。
- `DELETE /me/identities/{email|google|line}`：回更新後的 `/me`；最後登入方式回 `409 LAST_IDENTITY`。
- `GET /auth/oauth/{google|line}/start?link=true`：須已登入，完成後回 `/settings`，不換發 session；一般登入入口仍維持原行為。
- `0007_identity_linking.sql`：擴充 OTP 的 link 用途與帳號／session scope，API 啟動時自動套用。沒有新增環境變數或依賴。

## 停權與 OAuth 錯誤

OAuth 登入遇到停權時回 302 至 `/login?error=account_blocked`，帳號所有權衝突則至 `/login?error=identity_in_use`，不向瀏覽器顯示整頁 JSON。前端收到 `ACCOUNT_BLOCKED` 時清除使用者狀態及原生 token，導回登入頁並顯示停權原因；首次載入驗證也保留此原因。其他 403 不會清除登入狀態。

## 後台復原與唯讀模式

後台復原沿用 OTP 限流，驗證碼與稽核紀錄在同一筆交易內提交，提交後才背景寄信，不會持有交易時另取連線。等待 OTP 鎖後會重新核對帳號狀態與聯絡 Email；變更時回滾，不寄往舊信箱。

唯讀模式會擋住後台停權等帳號寫入操作，回 `503 READ_ONLY_MODE`，不修改帳號、session 或新增該操作的稽核。Staff 可先關閉唯讀旗標，再執行停權；旗標操作仍可使用。
