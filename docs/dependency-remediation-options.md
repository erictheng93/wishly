# 非 High／Critical 依賴處理決策

查核日期：2026-10-15。範圍：API 的 `rsa 0.9.10`，以及 Tauri 原生殼的 `glib 0.18.5`、`proc-macro-error 1.0.4`。本次未修改 manifest、lockfile 或依賴來源。

## 採用決策

| 項目 | 上游狀態 | 本次處理 |
| --- | --- | --- |
| `rsa`／RUSTSEC-2023-0071，Medium 5.9 | RustSec 尚無已確認修補版本 | 拋棄式副本重建仍含 RSA；保留原 lockfile 與 PostgreSQL 專用 feature 設定 |
| `glib`／RUSTSEC-2024-0429、GHSA-wrw7-89jp-8q8g，Moderate | 修復範圍 `>=0.20.0`；GTK 0.18 要求 `glib ^0.18` | 不建立 fork／vendor；CI 明列該 advisory ID 為暫時例外，待 Tauri 整鏈升級 |
| `proc-macro-error`／RUSTSEC-2024-0370，停止維護 | 無已知漏洞；原套件停止維護 | CI 明列該 advisory ID 為暫時例外，等待父套件移除依賴 |

上游公告：[RSA](https://rustsec.org/advisories/RUSTSEC-2023-0071.html)、[GLib](https://rustsec.org/advisories/RUSTSEC-2024-0429.html)、[proc-macro-error](https://rustsec.org/advisories/RUSTSEC-2024-0370.html)。

**GLib 問題仍未修補；上述例外是風險接受，不是修復證據。** 原生 `cargo audit` 在加入例外前已回傳 0（零漏洞、兩個 informational warnings），加入例外後仍回傳 0。這次修改是明確記錄政策，不是讓原本失敗的工作轉綠；其他 advisory 的掃描不受這兩個 ID 例外影響。

## RSA：重建結果與保留理由

API 的 `sqlx` 已設定 `default-features = false`，只啟用 PostgreSQL 等需要的功能。`cargo tree --manifest-path apps/api/Cargo.toml --locked --target all --invert rsa` 沒有依賴輸出。鎖檔中的來源是 optional 的 `sqlx-macros-core → sqlx-mysql → rsa`。[SQLx 巨集 manifest](https://github.com/launchbadge/sqlx/blob/v0.8.6/sqlx-macros-core/Cargo.toml)、[MySQL manifest](https://github.com/launchbadge/sqlx/blob/v0.8.6/sqlx-mysql/Cargo.toml)。

已在拋棄式目錄複製完全相同的 API `Cargo.toml`、加入最小 Rust target，且不複製舊 lockfile，執行：

```sh
cargo generate-lockfile --manifest-path <副本>/Cargo.toml
```

成功回傳 0。新舊 lockfile 都有 305 個 package，且都含 `rsa 0.9.10`；重建另造成 11 個無關套件版本變動，因此未採用，原 lockfile 未修改。供重現比對的 SHA-256：

- API manifest：`9620b01253e5a373baba75e8747093c0d41ed6761bb9a6b2a1867e2c4582eb4c`
- 原 lockfile：`045bfdda90907ed5ac5311a8ed95c43f231c0bd1e8259651f0f77cf75ef78e8f`
- 重建 lockfile：`1c56cd2e0b71e4493a3733d645df9a3fb243dfe85b366fccd6b2c8927c1ceb51`

沿用 `scripts/audit-api.sh`：只有所有平台實際依賴樹不含 RSA 時才忽略 `RUSTSEC-2023-0071`；若重新引入則恢復完整稽核。這是可達性限制，不能宣稱 RSA 上游漏洞已修復。不可手刪鎖檔區塊或直接改版本號；若未來需要 RSA 私鑰操作，須先重新評估時間差攻擊與安全實作。[官方 workaround](https://rustsec.org/advisories/RUSTSEC-2023-0071.html)。

## GLib：接受暫時風險，不維護 fork

Linux 依賴樹包含 `tauri → tauri-runtime／wry／tao／muda → gtk／webkit2gtk → glib 0.18.5`；macOS `aarch64-apple-darwin` 目標沒有 `glib`。尚未證實本應用或其間接依賴會呼叫受影響的 `VariantStrIter`；有依賴不等於已證實可利用，macOS 建置成功也無法驗證 Linux 路徑。

問題在 Rust binding 的 `VariantStrIter::impl_get`，不是升級系統 GLib 即可解決。官方將輸出指標改成 `let mut p` 與 `&mut p`，避免 C 函式透過不可變參照寫入造成未定義行為；最佳化建置可能崩潰。[官方修正 PR #1343](https://github.com/gtk-rs/gtk-rs-core/pull/1343)。

本次不回補，以避免 fork／vendor 維護成本及更換來源的供應鏈負擔；保留 crates.io 來源與 checksum。若日後必須提前回補，需要可審查的 diff、固定 commit（或版控 vendor）、每次依賴更新檢查適用性及移除條件，並在具備 GTK／WebKitGTK 的 Linux runner 以 release 最佳化驗證相關 iterator，再建置並啟動原生殼。保留 0.18.5 的回補仍會被版本式掃描器命中。

已執行 `cargo update --manifest-path apps/web/src-tauri/Cargo.toml -p glib -p glib-macros -p gtk -p gtk3-macros --dry-run`，結果為 `Locking 0 packages`；指定 `glib --precise 0.20.0 --dry-run --offline` 則因 GTK 0.18.2 要求 `glib ^0.18` 失敗，不能單獨升級。

## proc-macro-error 與整鏈升級

`glib-macros 0.18.5` 與 `gtk3-macros 0.18.2` 都要求 `proc-macro-error 1.0`，只加入另一套巨集套件不會移除舊路徑。`proc-macro-error2` 的儲存庫已於 2026-06-07 封存，本次不採用。[RustSec](https://rustsec.org/advisories/RUSTSEC-2024-0370.html)、[fork 封存狀態](https://github.com/GnomedDev/proc-macro-error-2)。

GTK 0.19.0 已於 2026-09-08 發布，採用 GLib 0.22、最低 Rust 1.92，新 GTK／GLib 巨集已移除舊 `proc-macro-error`。但查核時 Tauri [#16170](https://github.com/tauri-apps/tauri/pull/16170) 與 Wry [#1843](https://github.com/tauri-apps/wry/pull/1843) 仍是 Draft，需等待相容整鏈版本。[GTK 發布](https://github.com/gtk-rs/gtk3-rs/releases/tag/0.19.0)、[GTK manifest](https://github.com/gtk-rs/gtk3-rs/blob/0.19.0/Cargo.toml)、[GTK 巨集 manifest](https://github.com/gtk-rs/gtk3-rs/blob/0.19.0/gtk3-macros/Cargo.toml)、[GLib 巨集 manifest](https://github.com/gtk-rs/gtk-rs-core/blob/0.22/glib-macros/Cargo.toml)。

## 重新評估與移除例外

- 每次 Tauri／GTK 依賴更新都檢查 #16170 及實際依賴樹；上游 PR 合併本身不代表本專案已升級。
- 採用相容的整鏈版本後，確認 GLib 已進入修補範圍、所有舊 `proc-macro-error` 路徑消失，移除對應 ignore，重跑未忽略的 audit 及原生建置／測試。
- **發布 Linux 原生殼前必須重新評估 GLib 風險並在 Linux runner 驗證。** 本次未做 Linux 編譯或回補測試，也沒有宣稱 Linux 發布已通過驗收。

CI 的原生稽核命令現在只加入兩個指定 ID：

```sh
cargo audit --file apps/web/src-tauri/Cargo.lock \
  --ignore RUSTSEC-2024-0429 --ignore RUSTSEC-2024-0370
```
