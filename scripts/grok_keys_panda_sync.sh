#!/usr/bin/env bash
# 批量 extract keys（本机）→ scp 到 Panda → 可选 sync enabled
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
KEYS_LOCAL="${KEYS_LOCAL:-$ROOT/reports/pure_http_keys}"
PANDA_KEYS="${PANDA_KEYS:-/opt/tnexus/pure_http_keys}"
ACCOUNT_IDS="${ACCOUNT_IDS:-}"

usage() {
  echo "Usage: ACCOUNT_IDS='86 304 92' $0 extract|scp|sync|all"
  exit 1
}

extract_keys() {
  : "${ACCOUNT_IDS:?set ACCOUNT_IDS='86 304 ...'}"
  mkdir -p "$KEYS_LOCAL"
  for id in $ACCOUNT_IDS; do
    echo "== extract account $id =="
    python3 "$ROOT/scripts/extract_old_pool_session_keys.py" --account "$id" || echo "WARN: extract failed for $id"
  done
}

scp_keys() {
  ssh panda "mkdir -p $PANDA_KEYS"
  scp -r "$KEYS_LOCAL"/account_*.json "panda:$PANDA_KEYS/" 2>/dev/null || {
    echo "no keys to scp in $KEYS_LOCAL"
    exit 1
  }
  echo "scp done → panda:$PANDA_KEYS"
}

sync_enabled() {
  ssh panda "bash -lc 'source /opt/tnexus/.env && export GROK_DATABASE_URL && bash /root/TNexus/scripts/sync_grok_enabled_from_keys.sh --keys-dir $PANDA_KEYS --apply'"
}

cmd="${1:-}"
case "$cmd" in
  extract) extract_keys ;;
  scp) scp_keys ;;
  sync) sync_enabled ;;
  all) extract_keys; scp_keys; sync_enabled ;;
  *) usage ;;
esac
