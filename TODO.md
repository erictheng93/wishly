# 待辦

- [x] 認捐（點數眾籌）逐筆可見度設定 — [#3](https://github.com/erictheng93/wishsync/issues/3)
  - 已完成：migration 0006 回填並保留原有欄位；建立時套帳號預設或指定 visibility，逐筆 PATCH；個人頁、錢包與資料匯出支援逐筆可見度；轉投保留設定，匿名認捐在個人頁僅本人可見。

- [ ] FundSheet 增加認捐當下的「誰看得到」欄位（issue #3 後續）：目前套帳號預設，認捐後可到錢包修改；預設公開時，修改前可能已被看到。

- [ ] 上線前以真實憑證驗證 Google／LINE 登入與 S3 頭像轉存 — [#6](https://github.com/erictheng93/wishsync/issues/6)

- [ ] 頭像轉存後續（登入方式管理 ac8da6d）
  - 背景轉存每次登入都 spawn 一個任務，沒有並行上限；加 semaphore。
  - 綁定成功導回 /settings 時頭像可能尚未就緒，需重新整理才會出現。

- [ ] 安全與維運後續（登入方式管理）
  - link OTP 與登入 OTP 共用每 email 每小時 5 封額度：已登入者可對任一信箱耗盡額度，讓對方該小時無法用 OTP 登入。
  - 綁定流程沒有重新驗證身分（re-auth），需決定是否要求。
  - `delete_me` 取全域 advisory lock（staff 保護），刪帳號頻率低時無礙，量大再改。
