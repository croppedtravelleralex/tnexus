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
if ! grep -q '^ACCOUNTS_BACKEND=postgres' "$ENV" 2>/dev/null; then
  ensure_kv ACCOUNTS_BACKEND sqlite
  ensure_kv ACCOUNTS_DB /gptimage/data/accounts.db
fi
ensure_kv SCHEDULING_STATE_FILE /data/pool/scheduling_state.json
ensure_kv USAGE_EVENTS_FILE /data/pool/usage_events.ndjson
ensure_kv PIPELINE_EVENTS_FILE /data/pool/pipeline_events.ndjson
ensure_kv ACCOUNT_OPS_BASE http://127.0.0.1:9011
ensure_kv ACCOUNT_OPS_TOKEN "$TOKEN"
ensure_kv GPTIMAGE_ROOT /gptimage
ensure_kv GATEWAY_BASE http://127.0.0.1:8014
ensure_kv GPTIMAGE_BASE http://127.0.0.1:8014
ensure_kv IMAGE_RESPONSE_FORMAT url
ensure_kv IMAGE_PARALLEL_CONCURRENCY 8
ensure_kv IMAGE_STORE_PATH /data/images

echo "env patched (ACCOUNT_OPS_TOKEN set if new)"
