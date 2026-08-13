#!/usr/bin/env bash
# udeal 唯一代理：复测 OCR gate
set -euo pipefail
export GROK_LOCAL_PROXY="${GROK_LOCAL_PROXY:-http://127.0.0.1:18130}"
export PYTHONPATH=/tmp
cd /tmp
CRED_KEY=$(grep credentialEncryptionKey /opt/grok2api/config.yaml | head -1 | cut -d: -f2- | tr -d ' "')
export GROK_CREDENTIAL_KEY="$CRED_KEY"
python3 <<'PY'
import base64, json, os, sqlite3, sys
from pathlib import Path
sys.path.insert(0, '/tmp')
from cryptography.hazmat.primitives.ciphers.aead import AESGCM

def decrypt(enc_b64, key):
    pad = '=' * ((4 - len(enc_b64) % 4) % 4)
    raw = base64.b64decode(enc_b64 + pad)
    nonce, ct = raw[:12], raw[12:]
    return AESGCM(key).decrypt(nonce, ct, None).decode()

key = base64.b64decode(os.environ['GROK_CREDENTIAL_KEY'])
con = sqlite3.connect('file:/opt/grok2api/data/backend.db?mode=ro', uri=True)
row = con.execute('select encrypted_proxy_url from egress_nodes where id=110').fetchone()
con.close()
udeal = decrypt(row[0], key)
os.environ['GROK_UPSTREAM_PROXY'] = udeal
from grok_pure_http_client import GrokPureHttpClient, run_gate, load_canary

keys = json.loads(Path('/tmp/nancy_keys.json').read_text())
keys['email'] = 'nancybaker2jyy@yumail.co'
c = GrokPureHttpClient(keys, signer='python', upstream_proxy=udeal)
rl = c.request('POST', '/rest/rate-limits', json_body={})
print('rate_limits', rl.status_code, rl.text[:300])
up = c.upload_file(Path('/tmp/grok_ocr_probe.png'), mime='image/png')
print('upload', up)
if up.get('fileMetadataId'):
    ocr = c.chat_new('提取图中全部可见文字，若无文字则描述画面。', file_ids=[up['fileMetadataId']])
    print('ocr', json.dumps({k: ocr.get(k) for k in ['ok','http','kind','reply']}, ensure_ascii=False)[:500])
PY
