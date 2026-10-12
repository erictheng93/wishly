# Spike S1–S3 驗收門檻與測試規程（v1，2026-10-12）

**用途**：把「通過標準」在測試前量化並凍結。本文只定義門檻與做法，**不含任何測試結果**；結果與結論寫在 [spike-decisions.md](spike-decisions.md)。
**凍結規則**：第一筆正式測試開始前，把本文 commit 並在 decisions 記下該 commit hash。之後若調整門檻，必須新增版本並說明理由，舊結果不得用新門檻重判。
**依據**：PRD §10 D45、03 §14.3。PRD 只給定性標準，數字為本文提案，**由產品負責人確認後凍結**（見底部「待確認」）。

## S1：Cloudflare SSR 與 LINE / FB / iMessage OG 預覽

| 項目 | 門檻 |
|---|---|
| 平台 × 連結 | LINE、Facebook、iMessage 三平台 ×（一般清單、長中文標題＋有封面圖清單）共 6 組，**6/6** 必須同時顯示標題、描述、封面圖 |
| 新鮮度 | 每組用從未貼過的 slug（避免平台快取），貼出後 60 秒內出現預覽；清單改標題後換新 slug 重測 |
| 爬蟲 HTML | 以各平台爬蟲 UA 直接請求 `/s/{slug}`：HTTP 200、HTML 內含 `og:title`、`og:description`、`og:image`（絕對網址且可公開存取）；首位元組 p95 ≤ 2 s（冷啟動也計入，取 10 次含首次） |
| 受限清單 | friends / password 清單貼出後預覽為通用文案，**不得出現清單標題或封面** |

做法：
```sh
for ua in 'facebookexternalhit/1.1' 'Line/13.0.0' 'Mozilla/5.0 (iPhone) iMessage'; do
  curl -s -A "$ua" -o /tmp/s1.html -w "%{http_code} %{time_starttransfer}s\n" "https://<app>/s/<slug>"
  grep -o '<meta[^>]*property="og:[^>]*>' /tmp/s1.html
done
```
失敗備案（03 §14.3）：邊緣預渲染 OG 專用路由／Worker 對爬蟲 UA 回預渲染 HTML。切換後須重跑本表。

## S2：LINE 內建瀏覽器的身分存活率

**裝置矩陣（最低）**：iOS ≥ 2 台（含最新 iOS）、Android ≥ 2 台（含一台非 Pixel 品牌）；LINE 各用「當時最新版」，並至少 1 台用前一個大版本。記錄機型、OS、LINE 版本。

**每台裝置各做 5 輪下列情境**（共 ≥ 4 台 × 5 輪 × 2 情境 = 40 次）：

| 情境 | 步驟 | 成功定義 |
|---|---|---|
| A 重開連結 | 在聊天點分享連結 → 認領 → 關閉頁面 → 回聊天再點同一連結 → 進「我的認領」 | 不經 Email 恢復，仍看得到剛才的認領且可取消 |
| B 重啟 LINE | 認領後完全結束 LINE（從多工清除）→ 重開 LINE 點連結 → 進「我的認領」 | 同上 |

| 指標 | 門檻 |
|---|---|
| 身分存活率（A+B 合計） | **≥ 95%**（40 次最多 2 次失敗）；且每台裝置不得 0/5 |
| Email 恢復備案 | 對「存活失敗」或另做的 3 輪：以認領確認信的管理連結在新分頁恢復，**100% 成功**，且網址上的權杖在載入後被清除 |
| `openExternalBrowser=1` | 每個 OS 各 5 次：確實開啟系統瀏覽器；記錄「IAB 內的認領在系統瀏覽器看不到」是否符合預期（儲存空間不共用，屬已知行為，需寫入文案而非算失敗） |
| 儲存可用性 | 每台記錄 cookie、localStorage 在 IAB 內是否可讀寫（`ws_guest`、`ws_guest_token`），不得暴露權杖內容 |

失敗備案：存活率 < 95% → 認領成功頁與 sheet 把「寄管理連結到 Email」升為預設展開並提示；記錄決策後以備案重跑情境 A、B（需達標）。

## S3：SSE 經 Cloudflare → Cloud Run 的穩定度

**前置檢查（先記錄，這常是失敗主因）**：Cloud Run 請求逾時設定值（預設 300 s，需 ≥ 3600 s 或明確接受重連）、CPU 是否在無請求時被節流、是否設 `min-instances`、Cloudflare 是否對該路由啟用緩衝／壓縮。伺服器本身每條連線最長 30 分鐘（`dashboard.rs`），所以**至少要連續跑 120 分鐘**才能看到 ≥ 3 次正常重連。

用 `scripts/sse-probe.mjs`，在**目標鏈路**上同時跑兩個探針：辦公室有線網路 1 個、手機行動網路（tethering）1 個；測試清單用大量數量的品項。

```sh
API_BASE=https://<api-host> SLUG=<slug> ITEM_ID=<item uuid> \
DURATION_MIN=120 TRIGGER_EVERY_S=60 OUT=spike-s3-<net>.jsonl node scripts/sse-probe.mjs
```

| 指標 | 門檻（每個探針） |
|---|---|
| 首位元組（TTFB） | p95 ≤ 2 s，max ≤ 5 s |
| 位元組間隔 | 任何兩個位元組間隔 **0 次 > 30 s**（心跳為 15 s） |
| 非預期斷線 | ≤ 1 次 / 小時；伺服器 30 分鐘上限造成的關閉不計入 |
| 重連 | 每次斷線後 ≤ 10 s 重新取得 200（retry 5 s + 連線） |
| 事件延遲（認領 POST 完成 → 收到 `item.updated`） | p50 ≤ 2 s、p95 ≤ 5 s、max ≤ 10 s |
| 遺漏事件 | **0**（10 秒內沒收到即算遺漏）；重連後頁面資料須與 `GET /public/wishlists/{slug}` 一致 |
| 前景／背景（人工，各 1 次） | iOS Safari 與 LINE IAB：切背景 5 分鐘再回前景，30 秒內資料收斂（SSE 或輪詢） |
| 降級路徑（人工） | 開飛航模式 30 秒再恢復：連續失敗 2 次後切輪詢（15 s），恢復後 ≤ 20 s 資料收斂 |

失敗備案：改輪詢（`useWishlistEvents` 已有）。切換後量測輪詢的資料收斂延遲 p95 ≤ 20 s，並記錄 Cloud Run 請求成本影響。

## 證據規格（每項 Spike 都要有）

放在 `docs/qa/evidence/<spike>-<yyyymmdd>/`（大檔另存並附連結）：原始紀錄（`.jsonl`、curl 輸出）、截圖（可遮蔽暱稱、Email、權杖）、裝置與版本表、部署 commit 與 Cloud Run／Cloudflare 設定截圖。**不得包含 session、guest 或存取權杖。**

## 待確認（產品負責人）

1. S2 存活率 95%、樣本 40 次是否可接受（PRD 未給數字）。
2. S3 的延遲 p95 ≤ 5 s 是否符合產品期望（PRD 未量化）。
3. 實測用的 Cloudflare 帳號、API 網域與 Cloud Run 專案由誰提供；iOS／Android 裝置清單由誰提供。
