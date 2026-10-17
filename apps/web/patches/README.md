# 暫時性依賴安全補丁

`npm ci`／`npm install` 的 `postinstall` 會先用 Git 套用 `dependency-security.patch`，再執行 `nuxt prepare`。不需新增 npm 套件；開發及建置環境須安裝 Git。安裝腳本檢查套件版本和修改前後的 SHA-256，允許重跑，遇到版本或原始碼變動則拒絕安裝，要求重新審查補丁。

- `braces@3.0.3`：[GHSA-vfj7-8cjw-p6xm](https://github.com/advisories/GHSA-vfj7-8cjw-p6xm)。解析器及 compile／expand／stringify 的 AST 走訪限制深度為 100（含終端節點），過深輸入拋出 `SyntaxError`。同時處理括號、直接傳入的 AST 和循環子節點；引號、跳脫字元、方括號內的字面內容維持原行為。原套件為 Jon Schlinkert 的 MIT 授權。
- `node-forge@1.4.0`：[GHSA-86w9-cpqp-85rv](https://github.com/advisories/GHSA-86w9-cpqp-85rv)。RSA PKCS#1 v1.5 驗證需完全消耗巢狀 DigestAlgorithm 元素，只允許 OID 與可選的空 NULL；拒絕多餘元素、以其他型別代替 NULL、非空 NULL。元素數量修正參考尚未合併的 [upstream PR #1152](https://github.com/digitalbazaar/forge/pull/1152)，另補上非空 NULL 的防護。不修改共用 ASN.1 驗證器或其他簽章方案。原套件為 Digital Bazaar 的 BSD-3-Clause／GPL-2.0 雙授權，沿用 BSD-3-Clause。

`npm test -- tests/dependencySecurity.test.ts` 驗證實際間接依賴的惡意輸入、正常 glob／簽章／憑證，以及補丁的全新套用、重跑與變動拒絕。`node scripts/patch-dependencies.mjs --check` 只核對版本與已套用的 hash，不會自動補上缺失補丁。安裝時不可使用 `--ignore-scripts`，否則不會套用補丁。

CI 的 `npm run audit:security` 執行原始 `npm audit --audit-level=high --json`，保存完整 `npm-audit.json` 並傳回 npm 的 exit code。上述兩個公告及其依賴鏈不設例外，本地補丁通過驗證也不會把 audit 失敗轉成成功。`npm ci` 的版本／hash 檢查與 `test:coverage` 中的安全回歸測試仍驗證暫時防護；`web-audit` 獨立保留失敗訊號及每週重查，上游正式修補並使原始 audit 清零前不列為 required check。

正式修補版發布後，更新 lockfile，重新跑上述回歸測試及 typecheck／test:coverage／build／E2E，確認安全性與相容性後，移除對應補丁、版本／hash 檢查及最後不再需要的 postinstall 步驟。追蹤：[issue #7](https://github.com/erictheng93/wishly/issues/7)。

## 2026-10-17 關閉條件查核

結論：**issue #7 仍不可關閉**。兩個公告的 `Patched versions` 都是 `None`；`npm view braces version` 為 `3.0.3`，`npm view node-forge version` 為 `1.4.0`。本地補丁是暫時防護，不能替代正式發版與原始 audit 清零。

根因有兩層：braces 的遞迴 AST 走訪未限制深度，可造成 stack exhaustion；node-forge 的 RSA PKCS#1 v1.5 驗證未完全消耗巢狀 DigestAlgorithm 元素，可接受不合法簽章結構。現行 lockfile 由 Nuxt 4.6.1 → `@nuxt/nitro-server` → Nitro 2.13.4，分別經 globby → fast-glob／micromatch → braces，以及 listhen → node-forge 引入。原始 audit 的 11 個 high 是兩個公告及依賴鏈的傳播，並非 11 個獨立公告。registry 最新 Nuxt 4.6.2 的 `@nuxt/nitro-server` 仍使用 `nitropack@^2.13.4`，升級這個 patch 版無法取得尚未發布的修補版本，因此本次未更動 lockfile。

另外，原 `evaluateAudit` 會在本地補丁驗證成功後將這兩個公告的 npm exit 1 轉為 0，違反 issue 要保留失敗訊號的要求。本次移除此例外，改由 npm 決定掃描結果；回歸測試已先證實舊實作對這 11 項回傳 0，再驗證修正後保留 exit 1 與完整 JSON。安全補丁及其測試繼續保留。

以下命令由 `apps/web` 執行，環境為 Node 22.23.3、npm 11.19.0：

| 條件／命令 | 本次結果 |
| --- | --- |
| 兩個 advisory 正式修補版 | 未達成；兩者均尚未發布 |
| `npm ci` | 通過，postinstall 自動套用並核對補丁 |
| `npm audit --audit-level=high --json` | exit 1；11 high、0 critical |
| `npm run audit:security` | exit 1；保留完整 `npm-audit.json`，不豁免已回補公告 |
| `npm run typecheck` | 通過 |
| `npm run test:coverage` | 19 個測試檔、141 項通過；既有覆蓋率門檻通過 |
| `npm run build` | Cloudflare Pages production build 通過 |
| `npm run e2e -- guest-journey.spec.ts crowdfund.spec.ts` | 5 項通過；桌面／手機／平板訪客與兩項眾籌流程 |
| required check 再評估 | 暫不提升；GitHub 查核 main protection 回傳 404（未保護），rulesets 為空陣列 |

E2E 第一次因既有本機資料卷只有改名前的 `wishsync` 角色而無法啟動；補齊 compose 已約定的本機 `wishly` 開發角色後，重跑上述五項全部通過，未修改產品程式。未執行完整 E2E 或完整後端測試；本次執行範圍為 issue 指定的前端檢查與兩組 E2E。GitHub 遠端 CI 尚未驗證本次本機提交。

後續須等兩個上游正式修補版本發布，更新並驗證 Nuxt／Nitro 的 lockfile 解析，確認原始 audit high／critical 清零，再重跑安全與相容性檢查、移除暫時補丁並重新評估 required check。每週 `web-audit` 與 `if: always()` 原始 JSON artifact 持續保留；本次不以例外、偽造版本或 Nuxt 降版宣稱達成關閉條件。
