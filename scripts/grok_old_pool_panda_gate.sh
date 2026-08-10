#!/usr/bin/env bash
# Panda 老池 pure_http gate（keys 需预先放到 GROK_KEYS_DIR，禁止 build）
set -euo pipefail

KEYS_DIR="${GROK_KEYS_DIR:-/opt/tnexus/pure_http_keys}"
SCRIPT="${GROK_GATE_SCRIPT:-/root/TNexus/scripts/grok_pure_http_client.py}"
IMAGE="${GROK_OCR_PROBE_IMAGE:-/tmp/grok_ocr_probe.png}"
ACCOUNTS="${1:-86,304}"

python3.12 -m pip install -q curl_cffi cryptography 2>/dev/null || true

if [[ ! -f "$SCRIPT" ]]; then
  echo "missing $SCRIPT — git pull /root/TNexus first" >&2
  exit 1
fi
mkdir -p "$KEYS_DIR"

# 从 .env 加载 GROK_UPSTREAM_PROXY（Panda udeal 出口）
if [[ -f /opt/tnexus/.env ]]; then
  set -a
  # shellcheck disable=SC1091
  source /opt/tnexus/.env
  set +a
fi

export GROK_KEYS_DIR="$KEYS_DIR"
export GROK_TNEXUS_ROOT="${GROK_TNEXUS_ROOT:-/root/TNexus}"
export PYTHONPATH="$(dirname "$SCRIPT")"

IFS=',' read -ra IDS <<< "$ACCOUNTS"
ok=0
n=0
for id in "${IDS[@]}"; do
  id="${id// /}"
  [[ -z "$id" ]] && continue
  n=$((n + 1))
  keys="$KEYS_DIR/account_${id}.json"
  if [[ ! -f "$keys" ]]; then
    echo "{\"account_id\":$id,\"ok\":false,\"error\":\"missing $keys\"}"
    continue
  fi
  echo "=== gate account $id ==="
  if python3.12 "$SCRIPT" --keys "$keys" --gate --signer auto --image "$IMAGE"; then
    ok=$((ok + 1))
  fi
done
echo "{\"ok\":$ok,\"n\":$n,\"keys_dir\":\"$KEYS_DIR\"}"
exit $([[ "$ok" -eq "$n" && "$n" -gt 0 ]] && echo 0 || echo 1)
