#!/usr/bin/env bash
# Panda 一键：grok2api-rs 临时起服 + 纯 HTTP chat 冒烟（udeal 出口）
set -euo pipefail

PORT="${GROK_TEST_PORT:-18000}"
AUTH_KEY="${GROK_GATEWAY_AUTH_KEY:-grok-smoke-test}"
CONTAINER="grok2api-rs-smoke"

cleanup() { docker rm -f "$CONTAINER" 2>/dev/null || true; }
trap cleanup EXIT

source /opt/tnexus/.env
CRED_KEY="$(grep credentialEncryptionKey /opt/grok2api/config.yaml | head -1 | cut -d: -f2- | tr -d ' \"')"
if [[ -z "${CRED_KEY}" || -z "${DATABASE_URL:-}" ]]; then
  echo "missing CRED_KEY or DATABASE_URL" >&2
  exit 2
fi

GROK_DB="${DATABASE_URL}"

docker rm -f "$CONTAINER" 2>/dev/null || true
docker run -d --name "$CONTAINER" --network host \
  -e RUST_LOG=info \
  -e "GROK2API_ADDR=0.0.0.0:${PORT}" \
  -e "GROK_DATABASE_URL=${GROK_DB}" \
  -e "GROK_CREDENTIAL_KEY=${CRED_KEY}" \
  -e GROK2API_DIRECT=1 \
  -e GROK2API_SIGNER_MODE=local \
  -e 'GROK2API_PROXY_LIST=127.0.0.1:18130' \
  -e 'GROK_LOCAL_PROXY=http://127.0.0.1:18130' \
  -e "GROK_GATEWAY_AUTH_KEY=${AUTH_KEY}" \
  ghcr.io/croppedtravelleralex/grok2api-rs:latest

for i in $(seq 1 30); do
  if curl -sf "http://127.0.0.1:${PORT}/readyz" >/dev/null 2>&1; then
    echo "readyz OK"
    break
  fi
  sleep 1
done
curl -sf "http://127.0.0.1:${PORT}/readyz" || { docker logs "$CONTAINER" 2>&1 | tail -30; exit 1; }

echo "--- chat completions smoke ---"
curl -sS -w "\nHTTP=%{http_code}\n" \
  -H "Authorization: Bearer ${AUTH_KEY}" \
  -H "Content-Type: application/json" \
  -d '{"model":"grok-chat","messages":[{"role":"user","content":"Reply OK only"}],"stream":false}' \
  "http://127.0.0.1:${PORT}/v1/chat/completions" | head -c 2000

echo ""
echo "--- docker logs tail ---"
docker logs "$CONTAINER" 2>&1 | tail -15
