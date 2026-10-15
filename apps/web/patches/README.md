# 暫時性依賴安全補丁

`npm ci`／`npm install` 的 `postinstall` 會先用 Git 套用 `dependency-security.patch`，再執行 `nuxt prepare`。不需新增 npm 套件；開發及建置環境須安裝 Git。安裝腳本檢查套件版本和修改前後的 SHA-256，允許重跑，遇到版本或原始碼變動則拒絕安裝，要求重新審查補丁。

- `braces@3.0.3`：[GHSA-vfj7-8cjw-p6xm](https://github.com/advisories/GHSA-vfj7-8cjw-p6xm)。解析器及 compile／expand／stringify 的 AST 走訪限制深度為 100（含終端節點），過深輸入拋出 `SyntaxError`。同時處理括號、直接傳入的 AST 和循環子節點；引號、跳脫字元、方括號內的字面內容維持原行為。原套件為 Jon Schlinkert 的 MIT 授權。
- `node-forge@1.4.0`：[GHSA-86w9-cpqp-85rv](https://github.com/advisories/GHSA-86w9-cpqp-85rv)。RSA PKCS#1 v1.5 驗證需完全消耗巢狀 DigestAlgorithm 元素，只允許 OID 與可選的空 NULL；拒絕多餘元素、以其他型別代替 NULL、非空 NULL。元素數量修正參考尚未合併的 [upstream PR #1152](https://github.com/digitalbazaar/forge/pull/1152)，另補上非空 NULL 的防護。不修改共用 ASN.1 驗證器或其他簽章方案。原套件為 Digital Bazaar 的 BSD-3-Clause／GPL-2.0 雙授權，沿用 BSD-3-Clause。

`npm test -- tests/dependencySecurity.test.ts` 驗證實際間接依賴的惡意輸入、正常 glob／簽章／憑證，以及補丁的全新套用、重跑與變動拒絕。套件版本維持原值，因此 `npm audit --audit-level=high` 仍會報警並使 `web-audit` 失敗；沒有忽略公告、改版號或放寬 CI。安裝時不可使用 `--ignore-scripts`，否則不會套用補丁。

正式修補版發布後，更新 lockfile，重新跑上述回歸測試及 typecheck／test:coverage／build／E2E，確認安全性與相容性後，移除對應補丁、版本／hash 檢查及最後不再需要的 postinstall 步驟。追蹤：[issue #7](https://github.com/erictheng93/wishsync/issues/7)。
