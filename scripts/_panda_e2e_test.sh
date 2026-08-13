#!/usr/bin/env bash
set -euo pipefail
source /opt/tnexus/.env
export GROK_GATEWAY_AUTH_KEY

echo "=== pool: enable only 86,304 ==="
psql "$GROK_DATABASE_URL" -c "UPDATE grok_accounts SET enabled = (id IN (86, 304)) WHERE provider = 'grok_web';"
psql "$GROK_DATABASE_URL" -c "SELECT id, enabled FROM grok_accounts WHERE id IN (86,304,92) ORDER BY id;"

echo "=== curl /v1/chat/completions ==="
payload='{"model":"grok-2","messages":[{"role":"user","content":"Reply with exactly: PONG"}],"stream":false}'
code=$(curl -s -o /tmp/grok_e2e.json -w '%{http_code}' \
  http://127.0.0.1:8000/v1/chat/completions \
  -H "Authorization: Bearer ${GROK_GATEWAY_AUTH_KEY}" \
  -H "Content-Type: application/json" \
  -d "$payload")
echo "http=$code"
cat /tmp/grok_e2e.json
echo

if [ "$code" = "200" ]; then
  grep -q PONG /tmp/grok_e2e.json && echo "PASS: reply contains PONG" || echo "WARN: 200 but no PONG in body"
else
  echo "FAIL"
  docker logs panda-grok2api-rs-1 2>&1 | tail -5
  exit 1
fi
