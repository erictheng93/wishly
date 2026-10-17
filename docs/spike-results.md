# Spike S1–S3 驗收門檻與結果紀錄

門檻沿用 `git show 5c13bb9:docs/qa/spike-protocol.md`（v1，2026-10-12，詳細做法與備案見該檔）。**門檻尚待產品負責人確認**（原規程「待確認」：S2 存活率 95%／樣本 40 次、S3 延遲 p95 ≤ 5 s、測試帳號與裝置由誰提供）；確認後於此記錄日期並凍結，之後調整須新增版本並說明理由，舊結果不得用新門檻重判。
受測鏈路（D49）：Cloudflare Pages `wishly.tw` ／ `api.wishly.tw` → Cloudflare Tunnel → VPS（Dokploy／Traefik）→ API。
**狀態：全部未執行。** 結果欄空白即無證據，不得推定通過。證據（`.jsonl`、截圖）放 `docs/spike-evidence/<spike>-<yyyymmdd>/`，不得含 session／guest／存取權杖。

## 門檻摘要

| Spike | 門檻 |
|---|---|
| S1 OG 預覽 | LINE、FB、iMessage ×（一般清單、長中文標題＋封面圖清單）共 6 組，**6/6** 顯示標題、描述、封面圖；每組用新 slug，60 秒內出現預覽。爬蟲 UA 請求 `/s/{slug}`：200、含 `og:title/description/image`（image 為公開絕對網址）、TTFB p95 ≤ 2 s（10 次含冷啟動）。friends／password 清單預覽不得出現標題或封面。 |
| S2 LINE IAB 身分存活 | ≥4 台（iOS ≥2、Android ≥2，含非 Pixel）× 5 輪 × 情境 A 重開連結／B 重啟 LINE；存活率 **≥95%**（40 次最多失敗 2）且每台不得 0/5；Email 管理連結恢復 100%；`openExternalBrowser=1` 每 OS 5 次；記錄 cookie／localStorage 可用性。 |
| S3 SSE | `scripts/sse-probe.mjs` 連續 ≥120 分鐘，有線＋行動網路各 1 探針。TTFB p95 ≤ 2 s／max ≤ 5 s；位元組間隔 0 次 > 30 s；非預期斷線 ≤ 1／小時；重連 ≤ 10 s；事件延遲 p50 ≤ 2 s／p95 ≤ 5 s／max ≤ 10 s；遺漏事件 0；背景 5 分鐘回前景 30 秒內收斂；飛航 30 秒後輪詢降級並於 ≤ 20 s 收斂。 |

S3 測前須記錄：cloudflared 版本與設定、Traefik read／write／idle 逾時、各層是否緩衝或壓縮 `text/event-stream`、Cloudflare 該路由的緩衝／壓縮設定、VPS 的 FD／記憶體上限。

## 總表

| Spike | 狀態 | 結論 | 採用方案 | 日期 | 決策者 |
|---|---|---|---|---|---|
| S1 | 未執行 | — | — | — | — |
| S2 | 未執行 | — | — | — | — |
| S3 | 未執行 | — | — | — | — |

通過條件：「通過」或「失敗＋備案已驗證」。只有這兩種才能解除 P0 阻擋。

## 每項紀錄模板

```
### S? 名稱
- 測試日期／測試者：
- 部署 commit、網域、Traefik／Tunnel 逾時、Cloudflare 路由設定：
- 環境與版本（裝置、OS、LINE 版本、網路）：
- 樣本數與原始證據連結：
- 各指標實測值 vs 門檻（逐列）：
- 結論：通過／失敗
- 若失敗：採用備案與備案驗證結果：
- 殘留風險與後續：
```

## 2026-10-17 前置檢查（非 Spike 結果）

- `https://api.wishly.tw/healthz`、`/readyz`、`https://wishly.tw/` 皆 200。
- 不存在的 slug：`/s/{slug}` 與 SSE 端點皆 404，尚未有真實測試清單可驗 OG／SSE。
