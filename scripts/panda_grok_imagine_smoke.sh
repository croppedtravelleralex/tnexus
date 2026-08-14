#!/usr/bin/env bash
# Panda / 本机：Lite 生图冒烟（直连 :8000，不经 NewAPI）。
# 成功：HTTP 200 + data[].url 或 b64_json；失败打印 body。
set -euo pipefail

BASE="${GROK_SMOKE_BASE:-http://127.0.0.1:8000}"
ENV_FILE="${ENV_FILE:-/opt/tnexus/.env}"
TIMEOUT="${GROK_IMAGINE_SMOKE_TIMEOUT:-150}"
PROMPT="${GROK_IMAGINE_SMOKE_PROMPT:-a simple red circle on white background}"

if [[ -f "$ENV_FILE" ]]; then
  KEY=$(grep '^GROK_GATEWAY_AUTH_KEY=' "$ENV_FILE" | cut -d= -f2- || true)
fi
KEY="${GROK_GATEWAY_AUTH_KEY:-${KEY:-}}"
if [[ -z "$KEY" ]]; then
  echo "GROK_GATEWAY_AUTH_KEY required" >&2
  exit 1
fi

echo "==> health ${BASE}/readyz"
curl -fsS -o /dev/null --max-time 5 "${BASE}/readyz"

echo "==> POST /v1/images/generations (Lite default, timeout=${TIMEOUT}s)"
body=$(curl -fsS --max-time "$TIMEOUT" \
  -H "Authorization: Bearer ${KEY}" \
  -H "Content-Type: application/json" \
  -d "{\"prompt\":\"${PROMPT}\",\"n\":1,\"response_format\":\"url\",\"size\":\"1024x1024\"}" \
  "${BASE}/v1/images/generations" || true)

if [[ -z "$body" ]]; then
  echo "empty response or timeout" >&2
  exit 1
fi

echo "$body" | head -c 2000
echo ""

if echo "$body" | grep -qE '"url"|"b64_json"'; then
  echo "==> PASS"
  exit 0
fi

echo "==> FAIL: no image in response" >&2
exit 1
