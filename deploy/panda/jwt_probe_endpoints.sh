#!/usr/bin/env bash
# 探测 gateway 上可用于鉴权健康检查的轻量端点
set -uo pipefail
K=$(grep -E '^GATEWAY_AUTH_KEY=' /opt/tnexus/.env | cut -d= -f2- | tr -d '\r\n')
echo "key_len=${#K}"
for p in /v1/models /api/auth/me /api/me /health /readyz /api/quota /v1/dashboard/billing/subscription; do
  C=$(curl -sS -o /dev/null -w '%{http_code}' --max-time 8 -H "Authorization: Bearer $K" "http://127.0.0.1:8014$p" 2>/dev/null || echo ERR)
  B=$(curl -sS -o /dev/null -w '%{http_code}' --max-time 8 -H "Authorization: Bearer bogus.bogus.bogus" "http://127.0.0.1:8014$p" 2>/dev/null || echo ERR)
  echo "$p  with_key=$C  with_bogus=$B"
done
