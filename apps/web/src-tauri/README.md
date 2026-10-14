# WishSync Tauri shell

Build manually (no before*Command): `cd apps/web && npm run build:native`, then `cargo tauri dev|build` from this directory.
Custom commands: session_token_get / session_token_set / session_token_clear (OS keychain, service tw.wishsync.app, user session).
Android has no native keyring backend wired up (commands will error there); iOS uses Keychain.

## 開發 / 打包指令（在 apps/web 執行）

- `npm run tauri:dev`：啟動 `NUXT_NATIVE=1` 的 Nuxt dev server（:3000）並開 Tauri 視窗，載入 `tauri.dev.conf.json`（CSP 額外允許 `http://localhost:8080`）。
  後端需允許該來源並簽發權杖：`NATIVE_ORIGINS=http://localhost:3000,tauri://localhost`（dev 預設只含 tauri 三種，不含 :3000）。
- `npm run tauri:build`：`build:native` 後打包；正式 CSP 不含 localhost，`NUXT_PUBLIC_API_BASE` / `NUXT_PUBLIC_SITE_URL` 於建置時設定。
- 已驗證：`tauri build --debug --no-bundle` 可編譯並啟動不崩潰。**未驗證**：webview 內實際登入、剪貼簿、外部連結（無畫面存取，需人工在 `tauri:dev` 走一遍）。
