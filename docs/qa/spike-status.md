# Spike S1–S3 上線前證據查核

查核日期：2026-10-12。基準：本機 `main`，HEAD `ea576e7`，含當前工作目錄。

**結論：未找到三項 Spike 的真機結論或結案技術決策紀錄；均未驗收，列為 P0 上線阻擋項，補測與結案優先於新功能。** 這是證據查核結果，不能據此斷言團隊從未在 repo 外測過，也不代表測試失敗。本次未執行真機測試。

| Spike | 找到的證據 | 目前狀態 |
| --- | --- | --- |
| S1：Cloudflare SSR／LINE、FB、iMessage OG | PRD §3 的 D48 記錄 2026-10-08 本機 workerd／Wrangler 與模擬爬蟲 UA 驗證；當時明載未部署正式 Cloudflare 帳號、未測真實 LINE／FB 爬蟲。後續有預覽部署流程，未找到三平台真實預覽結果。 | 部分驗證；未驗收 |
| S2：LINE 內建瀏覽器存活率 | 探索 QA §(d) 明載僅 Chromium／UA 模擬，LINE storage 清除以腳本模擬。未找到裝置、LINE 版本、重開前後認領識別或存活率紀錄。 | 未找到真機結論；未驗收 |
| S3：Cloudflare → Cloud Run SSE | 探索 QA §(d) 只觀察到「即時更新中」，未做長時間斷線／輪詢降級實測。`5da8fb2` 增加 retry 事件與臨時預覽部署，沒有 Cloudflare → Cloud Run 的持續連線、重連或延遲測量結果。 | 未找到目標部署鏈路結論；未驗收 |

## 查核來源與範圍

- [PRD §3、§10](../01-prd.html)：D45 要求「三項 Spike 結論寫入技術決策紀錄」；D48 是平台選擇與本機驗證，不能替代三項結案。
- [使用者旅程 §13、§14.3](../03-wireframe-user-journey.html#s14)：列出通過標準、備案，並明載 Playwright 無法替代 LINE 真機驗證。
- [探索 QA §(d)](exploratory-report.md)：明列真實 Cloudflare、LINE storage、SSE 測試限制；本機 `apps/web/qa/out/` 的 JSONL／log 未找到真機結論。QA 腳本中的 `S1` 是狀態流程測項名稱，不是 Spike S1 結案。
- [README 臨時公開測試](../../README.md)：快速通道不支援 SSE；此路徑測到輪詢不能當作 S3 通過。具名通道可做額外驗證，但不能替代規格指定的 Cloud Run 鏈路。
- 本機全部可見 Git refs 的提交訊息、文件變更歷史與現有文件／產物索引；`5da8fb2` 只記錄 SSE 修正與預覽部署準備。
- GitHub 截至查核時可見的全部 issue／PR 本文及 issue／review 留言（issue #3，PR [#1](https://github.com/erictheng93/wishsync/pull/1)、[#2](https://github.com/erictheng93/wishsync/pull/2)、[#4](https://github.com/erictheng93/wishsync/pull/4)、[#5](https://github.com/erictheng93/wishsync/pull/5)）：未找到 Spike 結案證據。PR 的一般瀏覽器／mobile E2E 通過不能視為 LINE 真機通過。

未查核團隊聊天、個人手機或未提供的外部測試紀錄；若已有結果，應補入可追溯證據再驗收。

## 優先補測與結案

1. **先固定驗收條件與環境。** 使用真實 Pages 分享網址，S3 後端經 Cloudflare 到 Cloud Run；記錄部署 commit、路由與代理／逾時設定。確認 iOS／Android 的裝置與 OS／LINE 版本矩陣。現有規格沒有明定 S2 樣本數／存活率門檻，S3 連線持續時間／延遲上限也未量化，須在測前定案，不能測後挑門檻。
2. **S1：** 在 LINE／FB／iMessage 實際貼 `/s/{slug}`，保存標題、描述、封面預覽截圖及同網址的 SSR OG 輸出；依 §14.3 三平台全部符合才通過。
3. **S2：** 在真實 LINE 內建瀏覽器完成認領，關閉頁面、重開 LINE／連結，確認能找回同一認領；記錄 cookie、localStorage、`ws_guest` 與 `X-Guest-Token` 各自是否保留，另測 `openExternalBrowser=1`。各裝置記錄成功數／總次數，並驗證 Email 管理連結找回路徑；證據不得暴露 token 或個資。
4. **S3：** 在目標鏈路記錄長連線、事件送出／收到時間、斷網重連、背景／前景切換、Cloud Run 逾時及輪詢降級／狀態收斂結果，不能只看 UI 狀態文字。
5. **寫入技術決策紀錄再解除阻擋。** 每項記錄測試日期、測試者、環境／版本、樣本與原始證據連結、實測值對照門檻、通過／失敗結論、採用方案及理由。若失敗需採備案，應記錄決策並驗證備案；不能只因已有退路程式碼就視為結案。

本文件只記錄證據缺口與補測順序，不充當三項 Spike 的結案技術決策。

門檻與測試規程見 [spike-protocol.md](spike-protocol.md)；量測工具 `scripts/sse-probe.mjs`（S3）；結果與決策填寫於 [spike-decisions.md](spike-decisions.md)（目前全部未執行）。
