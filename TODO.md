# 待辦

- [ ] 認捐（點數眾籌）逐筆可見度設定 — [#3](https://github.com/erictheng93/wishsync/issues/3)
  - 現況：認捐只依捐助者帳號 `default_claim_visibility`，改預設會連帶影響舊認捐（`apps/api/src/profiles.rs:51`）。
  - 做法：比照 claims，`contributions` 加 `visibility share_level` 欄位 + INSERT trigger 套帳號預設、回填既有資料；API 支援建立時指定與 PATCH；`profiles.rs` 改用逐筆欄位；前端認捐紀錄可調整。
