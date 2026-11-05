#!/usr/bin/env bash
# GHCR 上 wishly-api:latest 的 digest 一變，就請本機 Dokploy 重新部署 api。由 wishly-autodeploy.timer 每分鐘執行。
# 用輪詢而不用 webhook：Dokploy 管理介面不對外，API 金鑰權限是整個組織，不放進 GitHub（docs/deploy-runbook.md §1）。
# 設定檔（root 600）：GHCR_USER、GHCR_TOKEN（read:packages）、DOKPLOY_API_KEY、DOKPLOY_APP_ID。
set -euo pipefail
ENV_FILE=${WISHLY_AUTODEPLOY_ENV:-/etc/wishly-autodeploy.env}
STATE_DIR=${WISHLY_AUTODEPLOY_STATE:-/var/lib/wishly-autodeploy}
IMAGE=erictheng93/wishly-api
# shellcheck disable=SC1090
. "$ENV_FILE"
: "${GHCR_USER:?}" "${GHCR_TOKEN:?}" "${DOKPLOY_API_KEY:?}" "${DOKPLOY_APP_ID:?}"
DOKPLOY_URL=${DOKPLOY_URL:-http://localhost:3000}
mkdir -p "$STATE_DIR"
exec 9>"$STATE_DIR/lock"
flock -n 9 || exit 0 # 上一輪還在跑就略過

# 憑證一律經 stdin 交給 curl，不出現在 ps / 程序參數。
token=$(printf 'user = "%s:%s"\n' "$GHCR_USER" "$GHCR_TOKEN" |
  curl -fsS -K - "https://ghcr.io/token?scope=repository:$IMAGE:pull&service=ghcr.io" |
  sed -n 's/.*"token" *: *"\([^"]*\)".*/\1/p')
[ -n "$token" ] || { echo "取不到 GHCR token" >&2; exit 1; }
digest=$(printf 'Authorization: Bearer %s\n' "$token" |
  curl -fsSI -H @- \
    -H 'Accept: application/vnd.oci.image.index.v1+json, application/vnd.docker.distribution.manifest.list.v2+json, application/vnd.oci.image.manifest.v1+json, application/vnd.docker.distribution.manifest.v2+json' \
    "https://ghcr.io/v2/$IMAGE/manifests/latest" |
  tr -d '\r' | awk 'tolower($1) == "docker-content-digest:" { print $2 }')
case "$digest" in sha256:*) ;; *) echo "取不到 latest 的 digest：$digest" >&2; exit 1 ;; esac

last=$(cat "$STATE_DIR/deployed-digest" 2>/dev/null || true)
if [ -z "$last" ]; then
  # 安裝當下不部署：只記錄目前線上應該已在跑的 latest，下次 latest 移動才部署。
  echo "$digest" >"$STATE_DIR/deployed-digest"
  echo "首次執行，記錄目前的 latest：$digest"
  exit 0
fi
[ "${digest}" = "${last}" ] && exit 0

echo "latest 由 ${last} 換成 ${digest}，觸發 Dokploy 部署"
printf 'x-api-key: %s\n' "$DOKPLOY_API_KEY" |
  curl -fsS -o /dev/null -H @- -H 'Content-Type: application/json' -X POST \
    -d "{\"applicationId\":\"$DOKPLOY_APP_ID\",\"title\":\"autodeploy ${digest}\"}" \
    "$DOKPLOY_URL/api/application.deploy"
# 只有 Dokploy 接受後才記錄；失敗時下一分鐘重試。
echo "$digest" >"$STATE_DIR/deployed-digest"
