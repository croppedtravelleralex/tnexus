#!/usr/bin/env bash
# 本机 grok2api-rs + session keys 端到端（需 PG + GROK_CREDENTIAL_KEY）
# 用法：
#   export GROK_DATABASE_URL=postgres://...
#   export GROK_CREDENTIAL_KEY=...
#   export GROK_PURE_HTTP_KEYS_DIR=reports/pure_http_keys
#   export GROK_LOCAL_PROXY=http://127.0.0.1:7897
#   bash scripts/grok_local_e2e_chat.sh
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

: "${GROK_DATABASE_URL:?GROK_DATABASE_URL required}"
: "${GROK_CREDENTIAL_KEY:?GROK_CREDENTIAL_KEY required}"
export GROK_PURE_HTTP_KEYS_DIR="${GROK_PURE_HTTP_KEYS_DIR:-$ROOT/reports/pure_http_keys}"
export GROK2API_DIRECT=1
export GROK2API_SIGNER_MODE=native
export GROK_GATEWAY_AUTH_KEY="${GROK_GATEWAY_AUTH_KEY:-local-e2e-test-key}"
export GROK2API_ADDR="${GROK2API_ADDR:-127.0.0.1:18000}"
export GROK_ADMIN_LISTEN="${GROK_ADMIN_LISTEN:-127.0.0.1:18091}"
export GROK_ADMIN_SECRET="${GROK_ADMIN_SECRET:-12345678901234567890123456789012}"
export GROK_ADMIN_PASSWORD="${GROK_ADMIN_PASSWORD:-admin123456789012}"

cargo build -q -p grok2api-rs
./target/debug/grok2api-rs &
pid=$!
trap 'kill $pid 2>/dev/null || true' EXIT

for i in $(seq 1 30); do
  if curl -sf "http://${GROK2API_ADDR}/healthz" >/dev/null 2>&1; then
    break
  fi
  sleep 1
done

payload='{"model":"grok-2","messages":[{"role":"user","content":"Reply with exactly: PONG"}],"stream":false}'
code=$(curl -s -o /tmp/grok_local_e2e.json -w '%{http_code}' \
  "http://${GROK2API_ADDR}/v1/chat/completions" \
  -H "Authorization: Bearer ${GROK_GATEWAY_AUTH_KEY}" \
  -H "Content-Type: application/json" \
  -d "$payload")
echo "http=$code"
cat /tmp/grok_local_e2e.json
echo
test "$code" = "200"
