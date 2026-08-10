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
ensure_kv IMAGE_PARALLEL_CONCURRENCY 10
if grep -q '^IMAGE_PARALLEL_CONCURRENCY=0' "$ENV" 2>/dev/null; then
  sed -i 's/^IMAGE_PARALLEL_CONCURRENCY=0/IMAGE_PARALLEL_CONCURRENCY=10/' "$ENV"
fi
if grep -q '^IMAGE_PARALLEL_CONCURRENCY=8' "$ENV" 2>/dev/null; then
  sed -i 's/^IMAGE_PARALLEL_CONCURRENCY=8/IMAGE_PARALLEL_CONCURRENCY=10/' "$ENV"
fi
ensure_kv IMAGE_STORE_PATH /data/images

# Grok 子系统（grok2api-rs sidecar + tnexus-api 管理代理）
if ! grep -q '^GROK_DATABASE_URL=' "$ENV" 2>/dev/null && grep -q '^DATABASE_URL=' "$ENV" 2>/dev/null; then
  db="$(grep '^DATABASE_URL=' "$ENV" | cut -d= -f2-)"
  ensure_kv GROK_DATABASE_URL "$db"
fi
ensure_kv GROK2API_BASE http://127.0.0.1:8000
ensure_kv GROK_ADMIN_BASE http://127.0.0.1:8091
ensure_kv GROK_REDIS_URL redis://127.0.0.1:6380
ensure_kv GROK2API_DIRECT 1
ensure_kv GROK2API_SIGNER_MODE native
ensure_kv GROK_PURE_HTTP_KEYS_DIR /opt/tnexus/pure_http_keys
mkdir -p /opt/tnexus/pure_http_keys
# GROK_STATSIG_FINGERPRINT：本机 `python scripts/extract_statsig_fingerprint_local.py` 提取后写入 .env
if ! grep -q '^GROK_GATEWAY_AUTH_KEY=' "$ENV" 2>/dev/null; then
  ensure_kv GROK_GATEWAY_AUTH_KEY "$(openssl rand -hex 32)"
fi
if ! grep -q '^GROK_ADMIN_PASSWORD=' "$ENV" 2>/dev/null; then
  ensure_kv GROK_ADMIN_PASSWORD "$(openssl rand -hex 16)"
fi
if ! grep -q '^GROK_ADMIN_SECRET=' "$ENV" 2>/dev/null; then
  ensure_kv GROK_ADMIN_SECRET "$(openssl rand -hex 32)"
fi

echo "env patched (ACCOUNT_OPS_TOKEN set if new)"
