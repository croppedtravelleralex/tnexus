#!/usr/bin/env bash
set -euo pipefail
ENV=/opt/tnexus/.env
TOKEN=$(openssl rand -hex 24)

ensure_kv() {
  local key="$1" val="$2"
  if grep -q "^${key}=" "$ENV" 2>/dev/null; then
    return 0
  fi
  echo "${key}=${val}" >> "$ENV"
}

mkdir -p /opt/tnexus/data/pool
test -f "$ENV" || { echo "missing $ENV"; exit 1; }

if grep -q "^ACCOUNTS_FILE=" "$ENV" 2>/dev/null; then
  sed -i '/^ACCOUNTS_FILE=/d' "$ENV"
  echo "removed deprecated ACCOUNTS_FILE from $ENV"
fi

ensure_kv TNEXUS_ACCOUNT_OPS_IMAGE ghcr.io/croppedtravelleralex/tnexus-account-ops:latest
ensure_kv ACCOUNTS_DB /gptimage/data/accounts.db
ensure_kv SCHEDULING_STATE_FILE /data/pool/scheduling_state.json
ensure_kv USAGE_EVENTS_FILE /data/pool/usage_events.ndjson
ensure_kv ACCOUNT_OPS_BASE http://127.0.0.1:9011
ensure_kv ACCOUNT_OPS_TOKEN "$TOKEN"
ensure_kv GPTIMAGE_ROOT /gptimage

echo "env patched (ACCOUNT_OPS_TOKEN set if new)"
