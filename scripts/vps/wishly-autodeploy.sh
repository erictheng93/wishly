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
  curl -fsS --connect-timeout 10 --max-time 30 -K - "https://ghcr.io/token?scope=repository:$IMAGE:pull&service=ghcr.io" |
  sed -n 's/.*"token" *: *"\([^"]*\)".*/\1/p')
[ -n "$token" ] || { echo "取不到 GHCR token" >&2; exit 1; }
digest=$(printf 'Authorization: Bearer %s\n' "$token" |
  curl -fsSI --connect-timeout 10 --max-time 30 -H @- \
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

# Dokploy 的 application.deploy 只是排進佇列，部署之後仍可能失敗，所以不能在請求成功時就當作已部署。
# 以 deployment.all 找 title 為「autodeploy <digest>」最新的一筆，看到 done 才寫 deployed-digest。
dk() { printf 'x-api-key: %s\n' "$DOKPLOY_API_KEY" | curl -fsS --connect-timeout 10 --max-time 30 -H @- "$@"; }
trigger() {
  echo "觸發 Dokploy 部署：${last} -> ${digest}"
  dk -o /dev/null -H 'Content-Type: application/json' -X POST \
    -d "{\"applicationId\":\"$DOKPLOY_APP_ID\",\"title\":\"autodeploy ${digest}\"}" \
    "$DOKPLOY_URL/api/application.deploy"
  echo "$digest $(date +%s)" >"$STATE_DIR/pending-digest"
}
# 印出該 digest 最新一筆部署的 status（createdAt 為 ISO 字串，可直接字串比大小）；沒有就印 none。
status=$(dk "$DOKPLOY_URL/api/deployment.all?applicationId=$DOKPLOY_APP_ID" |
  python3 -I -c '
import json, sys
t = "autodeploy " + sys.argv[1]
d = [x for x in json.load(sys.stdin) if x.get("title") == t]
print(max(d, key=lambda x: str(x.get("createdAt")))["status"] if d else "none")' "$digest")
echo "digest ${digest} 的部署狀態：${status}"
case "$status" in
  done) echo "$digest" >"$STATE_DIR/deployed-digest"; rm -f "$STATE_DIR/pending-digest" ;;
  none | error | cancelled)
    # 同一 digest 距上次觸發未滿 10 分鐘就等：尚未出現紀錄，或壞映像失敗時不要每分鐘重部署
    { read -r p_digest p_at <"$STATE_DIR/pending-digest"; } 2>/dev/null || true
    if [ "${p_digest:-}" = "$digest" ] && [ $(($(date +%s) - ${p_at:-0})) -lt 600 ]; then
      echo "距上次觸發未滿 10 分鐘，等待"
    else
      trigger
    fi ;;
  *) ;; # running / queued / idle：等下一分鐘
esac
