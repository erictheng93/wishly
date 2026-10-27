#!/bin/sh
# 主機防火牆（第二層，Linode Cloud Firewall 是第一層）。用法：wishly-firewall.sh [on|off]，可重複執行。
# Tunnel（cloudflared）是出站連線，回應封包屬 ESTABLISHED，不受影響；本機 / SSH 通道（lo）也不受影響。
set -eu
IF=eth0
op=${1:-on}

rule() {
  t=$1; shift
  if [ "$op" = off ]; then
    while "$t" -D "$@" 2>/dev/null; do :; done
  else
    "$t" -C "$@" 2>/dev/null || "$t" -I "$@"
  fi
}

for t in iptables ip6tables; do
  # 1) Docker 對外發佈的埠（Traefik 80/443、Dokploy 3000，以及日後任何誤發佈的埠）：外部的新連線一律丟棄。
  #    Docker 發佈的埠走 FORWARD，ufw / INPUT 擋不到，只能寫在 DOCKER-USER。
  if "$t" -nL DOCKER-USER >/dev/null 2>&1; then
    rule "$t" DOCKER-USER -i "$IF" -m conntrack --ctstate NEW -j DROP
  fi
  # 2) 主機行程直接監聽的埠：IPv6 的 docker-proxy（走 INPUT）、Swarm 管理 2377 / gossip 7946 / overlay 4789（單節點不需對外）
  rule "$t" INPUT -i "$IF" -p tcp -m multiport --dports 80,443,3000,2377,7946 -j DROP
  rule "$t" INPUT -i "$IF" -p udp -m multiport --dports 443,4789,7946 -j DROP
done
