#!/usr/bin/env bash
# 枚举 gateway 上与账号/刷新相关的路由
set -uo pipefail
K=$(grep -E '^GATEWAY_AUTH_KEY=' /opt/tnexus/.env | cut -d= -f2- | tr -d '\r\n')
GW=http://127.0.0.1:8014

probe() {
  local m=$1 p=$2
  local c
  c=$(curl -sS -o /dev/null -w '%{http_code}' --max-time 6 -X "$m" \
        -H "Authorization: Bearer $K" -H 'Content-Type: application/json' \
        "$GW$p" 2>/dev/null || echo ERR)
  [[ "$c" != "404" && "$c" != "ERR" ]] && echo "$m $p -> $c"
}

for p in \
  /api/accounts /api/accounts/refresh /api/accounts/refresh-token /api/accounts/refresh_tokens \
  /api/accounts/token/refresh /api/accounts/refresh-all /api/accounts/bulk-refresh \
  /api/tokens/refresh /api/token/refresh /api/refresh /api/refresh-tokens \
  /api/ops/token-refresh /api/ops/tokens/refresh /api/ops/accounts/refresh \
  /api/nurture /api/nurture/status /api/ops/nurture/status \
  /api/quota/refresh /api/accounts/quota/refresh /api/scheduler /api/scheduler/state \
  /api/health /api/stats /api/metrics /metrics /api/config /api/routes ; do
  probe GET "$p"
  probe POST "$p"
done

echo "=== 单账号路径形态 ==="
EMAIL=$(curl -sS --max-time 10 -H "Authorization: Bearer $K" "$GW/api/accounts" 2>/dev/null \
  | python3 -c "import json,sys; d=json.load(sys.stdin); a=d if isinstance(d,list) else (d.get('accounts') or []); print(a[0]['email'] if a else '')")
echo "sample_email=$EMAIL"
for p in "/api/accounts/$EMAIL" "/api/accounts/$EMAIL/refresh" "/api/accounts/$EMAIL/token/refresh" \
         "/api/accounts/$EMAIL/refresh-token" "/api/account/$EMAIL/refresh"; do
  probe GET "$p"
  probe POST "$p"
done
