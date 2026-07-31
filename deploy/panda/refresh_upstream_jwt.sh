#!/usr/bin/env bash
# Refresh worker → gateway JWT in /opt/tnexus/.env (in-place; never rewrite the whole file).
set -euo pipefail

ENV_FILE="${ENV_FILE:-/opt/tnexus/.env}"
GATEWAY_ENV="${GATEWAY_ENV:-/root/gptimage-gateway-rs/secrets/gateway.env}"
GATEWAY_LOGIN_URL="${GATEWAY_LOGIN_URL:-http://127.0.0.1:8014/api/auth/login}"

if [[ ! -f "$ENV_FILE" ]]; then
  echo "missing $ENV_FILE" >&2
  exit 1
fi
if [[ ! -f "$GATEWAY_ENV" ]]; then
  echo "missing $GATEWAY_ENV" >&2
  exit 1
fi

if ! curl -fsS -o /dev/null --max-time 3 http://127.0.0.1:8014/health 2>/dev/null; then
  echo "gateway :8014 not healthy — skip UPSTREAM_API_KEY refresh" >&2
  exit 0
fi

PASS=$(grep '^AUTH_BOOTSTRAP_ADMIN_PASSWORD=' "$GATEWAY_ENV" | cut -d= -f2- || true)
if [[ -z "$PASS" ]]; then
  echo "AUTH_BOOTSTRAP_ADMIN_PASSWORD missing in $GATEWAY_ENV" >&2
  exit 1
fi

GW_TOKEN=$(
  curl -fsS -c - -X POST "$GATEWAY_LOGIN_URL" \
    -H "Content-Type: application/json" \
    -d "$(printf '{"username":"admin","password":"%s"}' "$PASS")" \
    -o /dev/null \
    | awk '/gws_session/ {print $NF}'
)

if [[ -z "$GW_TOKEN" ]]; then
  echo "failed to obtain gateway JWT from :8014" >&2
  exit 1
fi

tmp=$(mktemp)
grep -v '^UPSTREAM_API_KEY=' "$ENV_FILE" >"$tmp"
printf 'UPSTREAM_API_KEY=%s\n' "$GW_TOKEN" >>"$tmp"
mv "$tmp" "$ENV_FILE"
chmod 600 "$ENV_FILE"
echo "refreshed UPSTREAM_API_KEY (len=${#GW_TOKEN})"
