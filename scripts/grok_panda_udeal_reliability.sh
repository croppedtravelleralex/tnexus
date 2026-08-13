#!/usr/bin/env bash
# Panda 上运行：udeal 出口 HTTP 可靠性探测
set -euo pipefail
export PYTHONPATH=/tmp
export GROK_KEYS_DIR=/tmp/pure_http_keys
cd /tmp
python3 -m pip install -q curl_cffi cryptography 2>/dev/null || true

CRED_KEY=$(grep credentialEncryptionKey /opt/grok2api/config.yaml | head -1 | cut -d: -f2- | tr -d ' "')
export GROK_CREDENTIAL_KEY="$CRED_KEY"

export GROK_UPSTREAM_PROXY=$(python3 <<'PY'
import base64, os, re, sqlite3
from cryptography.hazmat.primitives.ciphers.aead import AESGCM
cfg = open("/opt/grok2api/config.yaml").read()
m = re.search(r'credentialEncryptionKey:\s*"([^"]+)"', cfg)
key = base64.b64decode(m.group(1))
def dec(s):
    pad = "=" * ((4 - len(s) % 4) % 4)
    raw = base64.b64decode(s + pad)
    return AESGCM(key).decrypt(raw[:12], raw[12:], None).decode()
con = sqlite3.connect("file:/opt/grok2api/data/backend.db?mode=ro", uri=True)
row = con.execute("select encrypted_proxy_url from egress_nodes where id=110").fetchone()
print(dec(row[0]))
PY
)

EMAIL="${1:-aharrisd00r@yumail.co}"
ROUNDS="${2:-3}"
KEYS="/tmp/pure_http_keys/${EMAIL/@/_at_}.json"
if [[ ! -f "$KEYS" ]]; then
  echo "missing keys: $KEYS" >&2
  exit 1
fi

python3 /tmp/grok_http_reliability_probe.py \
  --email "$EMAIL" \
  --keys "$KEYS" \
  --rounds "$ROUNDS" \
  --signer python \
  --image "${IMAGE:-/tmp/grok_ocr_probe.png}" \
  --json-out "/tmp/reliability_panda_${EMAIL/@/_at_}.json"
