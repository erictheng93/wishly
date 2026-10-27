#!/usr/bin/env bash
# SSH 加固（Ubuntu 24.04 / OpenSSH 9.6，wishly VPS）。以 root 執行；執行前保持現有 SSH 連線開著。
set -euo pipefail

# 0) 防鎖死：root 必須已有公鑰（目前 1 把，就是你 Mac 用的那把）
grep -qE '^(ssh-|ecdsa-|sk-)' /root/.ssh/authorized_keys || { echo "root 沒有公鑰，中止"; exit 1; }

# 1) sshd 同一設定取「第一個出現的值」；sshd_config 第 12 行先 Include 這個目錄，所以 drop-in 會蓋過主檔的 yes
cat >/etc/ssh/sshd_config.d/10-wishly-hardening.conf <<'EOF'
PermitRootLogin prohibit-password
PasswordAuthentication no
KbdInteractiveAuthentication no
PermitEmptyPasswords no
MaxAuthTries 3
LoginGraceTime 20
X11Forwarding no
EOF
chmod 644 /etc/ssh/sshd_config.d/10-wishly-hardening.conf

# 2) 語法檢查通過才套用；reload 不會斷掉現有連線
sshd -t
sshd -T | grep -E '^(permitrootlogin|passwordauthentication|kbdinteractiveauthentication|maxauthtries) '
systemctl reload ssh

# 3) fail2ban（金鑰登入後只是減少雜訊；Ubuntu 24.04 用 systemd journal 當來源最穩）
apt-get update -qq
DEBIAN_FRONTEND=noninteractive apt-get install -y -qq fail2ban python3-systemd
cat >/etc/fail2ban/jail.d/10-wishly.local <<'EOF'
[sshd]
enabled  = true
backend  = systemd
maxretry = 5
findtime = 10m
bantime  = 1h
ignoreip = 127.0.0.1/8 ::1
EOF
systemctl enable --now fail2ban
systemctl restart fail2ban
sleep 2
fail2ban-client status sshd
echo "完成。請在『另一個終端機』確認還能登入後，再關掉目前這個連線。"
