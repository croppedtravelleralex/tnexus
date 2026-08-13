#!/usr/bin/env bash
# Panda udeal 批量 gate（keys 需预先 scp 到 /tmp/pure_http_keys）
set -euo pipefail
export PYTHONPATH=/tmp
export GROK_KEYS_DIR=/tmp/pure_http_keys
cd /tmp
python3 -m pip install -q curl_cffi cryptography 2>/dev/null || true

CRED_KEY=$(grep credentialEncryptionKey /opt/grok2api/config.yaml | head -1 | cut -d: -f2- | tr -d ' "')
export GROK_CREDENTIAL_KEY="$CRED_KEY"
export GROK_UPSTREAM_PROXY=$(python3 <<'PY'
import base64, re, sqlite3
from cryptography.hazmat.primitives.ciphers.aead import AESGCM
cfg = open("/opt/grok2api/config.yaml").read()
m = re.search(r'credentialEncryptionKey:\s*"([^"]+)"', cfg)
key = base64.b64decode(m.group(1))
def dec(s):
    pad = "=" * ((4 - len(s) % 4) % 4)
    raw = base64.b64decode(s + pad)
    return AESGCM(key).decrypt(raw[:12], raw[12:], None).decode()
con = sqlite3.connect("file:/opt/grok2api/data/backend.db?mode=ro", uri=True)
print(dec(con.execute("select encrypted_proxy_url from egress_nodes where id=110").fetchone()[0]))
PY
)

EMAILS_FILE="${1:-/tmp/yumail_gate_emails.txt}"
python3 /tmp/grok_batch_yumail_gate.py \
  --emails-file "$EMAILS_FILE" \
  --skip-extract \
  --signer python \
  --image /tmp/grok_ocr_probe.png \
  --sleep 2
