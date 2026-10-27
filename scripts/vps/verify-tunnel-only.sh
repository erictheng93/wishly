#!/usr/bin/env bash
# 驗證 Wishly API 只能經 Cloudflare Tunnel 到達（docs/deploy-backend-checklist.md §2.3 / §2.5 / §7.5）。
# 用法：
#   外部檢查（在 VPS 以外的機器跑，例如你的 Mac）： ./verify-tunnel-only.sh external <VPS 公網 IPv4> [VPS 公網 IPv6]
#   主機檢查（ssh 進 VPS 後以 root 跑）：           ./verify-tunnel-only.sh host
#   封包取樣（ssh 進 VPS 後以 root 跑，最可靠）：     ./verify-tunnel-only.sh capture [秒數，預設 300]
# 只做連線嘗試與唯讀查詢，不改任何設定。
# 注意：部分網路（ISP / 公司防火牆）會攔截出站 80／443 並假裝握手成功，external 會誤報 OPEN；
# 先拿一個不相干的 IP 試同樣的埠，若也 OPEN 就以 capture 為準：網路上的掃描器會持續敲 22／80／443，
# 只看到 22 的 SYN、其他埠 0 筆，即表示外層防火牆擋下了其餘入站。
set -u
mode=${1:-}

external() {
  local fail=0
  for ip in "$@"; do
    [ -z "$ip" ] && continue
    local host=$ip; [[ $ip == *:* ]] && host="[$ip]"
    echo "== ${ip}：TCP 連線（應全部 closed/filtered）"
    for p in 80 443 3000 8080 5432 9000 9001 2375 2376 7946; do
      if nc -z -w 3 "$ip" "$p" 2>/dev/null; then echo "  !! $p OPEN"; fail=1; else echo "  ok $p closed/filtered"; fi
    done
    # 就算埠開著，也確認能不能用 Host 標頭打到 API 並偽造 CF-Connecting-IP（Traefik 依 Host 路由時的繞過路徑）
    for scheme in http https; do
      code=$(curl -sk -m 5 -o /dev/null -w '%{http_code}' -H 'Host: api.wishly.tw' -H 'CF-Connecting-IP: 203.0.113.7' \
        "$scheme://$host/healthz" 2>/dev/null)
      case $code in
        000) echo "  ok $scheme 無回應" ;;
        *) echo "  !! $scheme 回 HTTP $code：可繞過 Tunnel 直達（若為 200 即 API 本身）"; fail=1 ;;
      esac
    done
  done
  [ $fail = 0 ] && echo "PASS：外部無法直連" || echo "FAIL：見上方 !! 項"
}

host() {
  echo "== 對外監聽的埠（0.0.0.0 / [::] / 公網 IP 都算對外）"
  ss -tlnpH | awk '{print $4, $6}' | grep -vE '^(127\.|\[::1\]|::1)' || true
  echo "== Docker 對主機發佈的埠（Docker 會直接寫 iptables，ufw 擋不住這些）"
  docker ps --format '{{.Names}}\t{{.Ports}}' | grep -E '0\.0\.0\.0|:::' || echo "  （無）"
  echo "== Docker Swarm ingress 發佈（Dokploy 預設用 Swarm，Traefik 常以 ingress/host 模式發佈 80/443）"
  docker service ls --format '{{.Name}}\t{{.Ports}}' 2>/dev/null || true
  echo "== DOCKER-USER 鏈（要擋 Docker 發佈的埠必須寫在這裡）"
  iptables -S DOCKER-USER 2>/dev/null || echo "  （無 iptables 或無此鏈）"
  echo "== ufw"
  ufw status verbose 2>/dev/null || echo "  （未安裝）"
  echo "== cloudflared 是否在跑、ingress 指向哪"
  docker ps --format '{{.Names}}\t{{.Image}}' | grep -i cloudflared || pgrep -a cloudflared || echo "  !! 找不到 cloudflared"
  echo "== wishly-api 環境（只列 BIND / TRUSTED_PROXY）"
  for c in $(docker ps --format '{{.Names}}' | grep -i api); do
    echo "  $c: $(docker inspect "$c" --format '{{range .Config.Env}}{{println .}}{{end}}' | grep -E '^(BIND|TRUSTED_PROXY)=' | tr '\n' ' ')"
  done
  echo "（另請到 Linode Cloud Manager → Firewalls 確認此 Linode 已掛防火牆且 Inbound 預設 DROP、只放行你的 SSH 來源；它在主機外，Docker 繞不過）"
}

capture() {
  local secs=${1:-300}
  local me; me=$(ip -4 -br addr show eth0 | awk '{print $3}' | cut -d/ -f1)
  echo "== ${secs} 秒內到達 eth0（$me）的入站 SYN，依目的埠統計"
  # 只算目的是本機的 SYN；不加 dst host 會把容器 / cloudflared 的出站 443 也算進來
  timeout "$secs" tcpdump -nni eth0 -q "tcp[tcpflags] & tcp-syn != 0 and tcp[tcpflags] & tcp-ack == 0 and dst host $me" 2>/dev/null \
    | awk '{print $5}' | sed -E 's/.*\.([0-9]+):$/\1/' | grep . | sort | uniq -c | sort -rn
}

case $mode in
  external) shift; external "$@" ;;
  host) host ;;
  capture) shift; capture "$@" ;;
  *) echo "用法：$0 external <IPv4> [IPv6] | host | capture [秒數]"; exit 2 ;;
esac
