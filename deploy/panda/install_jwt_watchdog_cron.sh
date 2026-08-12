#!/usr/bin/env bash
# 安装 JWT 看门狗 cron：每 15 分钟探活一次，过期/401 立即刷新。
set -euo pipefail

CRON_FILE=/etc/cron.d/tnexus-jwt-watchdog
LOG=/var/log/tnexus-jwt-watchdog.log

cat >"$CRON_FILE" <<'EOF'
SHELL=/bin/bash
PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
*/15 * * * * root bash /root/TNexus/deploy/panda/jwt_watchdog.sh >>/var/log/tnexus-jwt-watchdog.log 2>&1
EOF
chmod 644 "$CRON_FILE"
touch "$LOG"

# 原每日 04:17 刷新改为每 6 小时一次，避免与 24h 过期时刻重合
cat >/etc/cron.d/tnexus-jwt-refresh <<'EOF'
SHELL=/bin/bash
PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
17 */6 * * * root cd /root/TNexus && bash /root/TNexus/deploy/panda/refresh_upstream_jwt.sh >>/var/log/tnexus-jwt-refresh.log 2>&1
EOF
chmod 644 /etc/cron.d/tnexus-jwt-refresh

systemctl reload cron 2>/dev/null || service cron reload 2>/dev/null || true

echo "installed:"
cat "$CRON_FILE"
echo "---"
cat /etc/cron.d/tnexus-jwt-refresh
