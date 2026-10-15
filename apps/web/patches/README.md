# 暫時性依賴安全補丁

`npm ci`／`npm install` 的 `postinstall` 會先用 Git 套用 `dependency-security.patch`，再執行 `nuxt prepare`。不需新增 npm 套件；開發及建置環境須安裝 Git。安裝腳本檢查套件版本和修改前後的 SHA-256，允許重跑，遇到版本或原始碼變動則拒絕安裝，要求重新審查補丁。

- `braces@3.0.3`：[GHSA-vfj7-8cjw-p6xm](https://github.com/advisories/GHSA-vfj7-8cjw-p6xm)。解析器及 compile／expand／stringify 的 AST 走訪限制深度為 100（含終端節點），過深輸入拋出 `SyntaxError`。同時處理括號、直接傳入的 AST 和循環子節點；引號、跳脫字元、方括號內的字面內容維持原行為。原套件為 Jon Schlinkert 的 MIT 授權。
- `node-forge@1.4.0`：[GHSA-86w9-cpqp-85rv](https://github.com/advisories/GHSA-86w9-cpqp-85rv)。RSA PKCS#1 v1.5 驗證需完全消耗巢狀 DigestAlgorithm 元素，只允許 OID 與可選的空 NULL；拒絕多餘元素、以其他型別代替 NULL、非空 NULL。元素數量修正參考尚未合併的 [upstream PR #1152](https://github.com/digitalbazaar/forge/pull/1152)，另補上非空 NULL 的防護。不修改共用 ASN.1 驗證器或其他簽章方案。原套件為 Digital Bazaar 的 BSD-3-Clause／GPL-2.0 雙授權，沿用 BSD-3-Clause。

`npm test -- tests/dependencySecurity.test.ts` 驗證實際間接依賴的惡意輸入、正常 glob／簽章／憑證，以及補丁的全新套用、重跑與變動拒絕。`node scripts/patch-dependencies.mjs --check` 只核對版本與已套用的 hash，不會自動補上缺失補丁。安裝時不可使用 `--ignore-scripts`，否則不會套用補丁。

CI 的 `npm run audit:security` 先執行原始 `npm audit --audit-level=high --json` 並保存完整 `npm-audit.json`。只有上述兩個公告的套件名稱、URL、受影響範圍、high 等級及已補丁安裝路徑完全符合，且所有 high 項目的依賴鏈都能追溯至它們時，才可能接受。接受前還必須通過唯讀版本／hash 檢查與安全回歸測試；額外未補丁副本、其他 high／critical 公告、公告範圍或等級變更、缺失依賴證據、掃描錯誤都會使 CI 失敗。原始版本警報仍保留於 artifact，已驗證的回補項目列在 job summary。這是有驗證前提的暫時例外，並非宣稱 npm registry 已發布修補版。

正式修補版發布後，更新 lockfile，重新跑上述回歸測試及 typecheck／test:coverage／build／E2E，確認安全性與相容性後，移除對應補丁、版本／hash 檢查及最後不再需要的 postinstall 步驟。追蹤：[issue #7](https://github.com/erictheng93/wishsync/issues/7)。
