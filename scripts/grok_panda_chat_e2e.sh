#!/usr/bin/env bash
# Panda 上探测 grok2api-rs /v1/chat/completions（只读 curl，不 build）
set -euo pipefail
ENV=/opt/tnexus/.env
test -f "$ENV" || { echo "missing $ENV"; exit 1; }
set -a
# shellcheck disable=SC1090
source "$ENV"
set +a
: "${GROK_GATEWAY_AUTH_KEY:?GROK_GATEWAY_AUTH_KEY missing}"

payload='{"model":"grok-2","messages":[{"role":"user","content":"Reply with exactly: PONG"}],"stream":false}'
code=$(curl -s -o /tmp/grok_chat_e2e.json -w '%{http_code}' \
  http://127.0.0.1:8000/v1/chat/completions \
  -H "Authorization: Bearer ${GROK_GATEWAY_AUTH_KEY}" \
  -H "Content-Type: application/json" \
  -d "$payload" || true)
echo "http=$code"
head -c 800 /tmp/grok_chat_e2e.json 2>/dev/null || true
echo
