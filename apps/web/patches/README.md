# 依賴安全 fork（已移除）

2026-10 曾以 `package.json` 的 `overrides` 把 `braces`、`node-forge` 換成自己維護的 fork（`erictheng93/braces`、`erictheng93/forge`，分支 `wishly-security`）。2026-11-16 兩者都符合原訂的移除條件，已刪除 override 並改回 npm 上游版本：

| 套件 | 公告 | 移除理由 |
| --- | --- | --- |
| `braces`（nitropack → globby → micromatch） | [GHSA-vfj7-8cjw-p6xm](https://github.com/advisories/GHSA-vfj7-8cjw-p6xm) | 公告已於 2026-10-23 撤銷。上游 3.0.3 沒有深度限制，但只在建置時處理專案內的 glob 樣式，不接受外部輸入。 |
| `node-forge`（nitropack → listhen） | [GHSA-86w9-cpqp-85rv](https://github.com/advisories/GHSA-86w9-cpqp-85rv) | 上游已發布修補版（受影響範圍 `<=1.4.0`，目前解析到 1.4.3）。 |

另一個直接原因：兩個 fork repo 已不存在，lockfile 指向的 git 來源無法安裝，CI 的 `npm ci` 會失敗。

`tests/dependencySecurity.test.ts` 保留 node-forge 的惡意簽章回歸測試，對 listhen 實際使用的副本驗證，上游若退版會立即失敗。braces 的回歸測試隨公告撤銷一併刪除。

之後若再需要修補間接依賴，優先等上游修補版或用 `overrides` 指定已發布版本；真的要 fork 時，應把原始碼放進本 repo（`file:` 來源），不要依賴外部 git repo。
