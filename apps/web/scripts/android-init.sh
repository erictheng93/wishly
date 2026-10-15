#!/usr/bin/env bash
# 初始化 Android 專案並強制關閉備份：權杖存在 App 私有目錄（src-tauri/src/file_store.rs），
# 若 allowBackup 為 true 會被雲端備份帶走。gen/ 是產物不進版控，所以每次 init 都要補這個設定。
# 用法：scripts/android-init.sh [manifest 路徑，測試用，會跳過 tauri android init]
set -euo pipefail
cd "$(dirname "$0")/.."
M="${1:-src-tauri/gen/android/app/src/main/AndroidManifest.xml}"
[ $# -ge 1 ] || npx tauri android init --ci
[ -f "$M" ] || { echo "找不到 $M" >&2; exit 1; }

if grep -q 'android:allowBackup=' "$M"; then
  perl -0pi -e 's/android:allowBackup="[^"]*"/android:allowBackup="false"/' "$M"
else
  perl -0pi -e 's/<application\b/<application android:allowBackup="false"/' "$M"
fi
# 另一份備份規則（Android 12+ 的 dataExtractionRules）不存在時，allowBackup=false 已足夠
grep -q 'android:allowBackup="false"' "$M" || { echo "allowBackup 未能關閉" >&2; exit 1; }
echo "OK: allowBackup=false ($M)"
