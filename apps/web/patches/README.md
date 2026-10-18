# 依賴安全 fork

`braces`、`node-forge` 的上游至今未發布修補版，所以改由 `package.json` 的 `overrides` 換成自己維護的 fork，固定在 commit SHA。兩者都是 Nuxt → `@nuxt/nitro-server` → nitropack 的間接依賴，只在建置／開發時執行，不會進入部署產物。

| 套件 | 公告 | Fork（分支 `wishly-security`） | 基準 | 版本 |
| --- | --- | --- | --- | --- |
| `braces`（nitropack → globby → micromatch） | [GHSA-vfj7-8cjw-p6xm](https://github.com/advisories/GHSA-vfj7-8cjw-p6xm) | [erictheng93/braces](https://github.com/erictheng93/braces/tree/wishly-security) | tag `3.0.3` | `3.0.4-wishly.1` |
| `node-forge`（nitropack → listhen） | [GHSA-86w9-cpqp-85rv](https://github.com/advisories/GHSA-86w9-cpqp-85rv) | [erictheng93/forge](https://github.com/erictheng93/forge/tree/wishly-security) | tag `v1.4.0` | `1.4.1-wishly.1` |

- `braces`：解析器及 compile／expand／stringify 的 AST 走訪限制深度為 100（含終端節點），過深輸入拋出 `SyntaxError`；也處理直接傳入的 AST 與循環子節點。引號、跳脫字元、方括號內的字面內容維持原行為。
- `node-forge`：RSA PKCS#1 v1.5 驗證要求巢狀 DigestAlgorithm 只含 OID 與可選的空 NULL，拒絕多餘元素、以其他型別代替 NULL、非空 NULL。參考尚未合併的 [upstream PR #1152](https://github.com/digitalbazaar/forge/pull/1152)，另補上非空 NULL 的防護。只改 `lib/rsa.js`。

每個 fork 只有兩個 commit：修補本身與版本號。版本號改成 prerelease 是為了讓 `npm audit` 不再比對到 `<=3.0.3`／`<=1.4.0`；若公告範圍日後擴大，audit 仍會報警。建立時已核對：tag 原始碼的 SHA-256 與 npm 發布的 3.0.3／1.4.0 相同；上游測試套件修補前後結果一致（braces 764 passing；forge 各 unit 檔通過數相同，`hmac`／`kem` 在原版即失敗）。授權沿用原套件（MIT；BSD-3-Clause／GPL-2.0 雙授權）。

驗證：`npm audit` 為 0（CI `web-audit` job，每週一排程重查；npm audit 對 git 來源同樣以名稱＋版本比對，新公告仍會報警）；`tests/dependencySecurity.test.ts` 對實際間接依賴的副本驗證惡意輸入被拒、正常 glob／簽章／憑證不受影響。移除 `overrides` 會讓這些測試失敗。

## 何時移除

任一條件成立就刪除對應的 override，跑 `npm install`、`npm audit`、`npm test`、`npm run build`：

- 上游發布修補版（`npm view braces version`／`npm view node-forge version` 超過 3.0.3／1.4.0），且 nitropack 的依賴範圍可取得該版本。
- `braces` 公告被撤銷（CVE 爭議處理中，見 [advisory-database#10132](https://github.com/github/advisory-database/pull/10132)）。
- nitropack／listhen 移除這兩個依賴。

修改 fork 時：在 `wishly-security` 分支新增 commit 並 push，再把 `package.json` 的 SHA 換成新的 commit。不要 force-push 或刪除舊 commit，否則既有 lockfile 會無法安裝。
