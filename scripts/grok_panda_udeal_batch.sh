#!/usr/bin/env bash
# Panda + udeal 出口：批量 gate + HTTP 可靠性探测（禁止 build）
set -euo pipefail

PANDA="${PANDA:-panda}"
LIMIT="${LIMIT:-5}"
ROUNDS="${ROUNDS:-3}"
EMAIL="${EMAIL:-aclarkdc8c@yumail.co}"
GROK_ROOT_LOCAL="${GROK_ROOT_LOCAL:-D:/SelfMadeTool/AutoRegister/grok/grok_bytao/grok_bytao}"
TNEXUS="${TNEXUS:-$(cd "$(dirname "$0")/.." && pwd)}"

echo "[panda-udeal] sync scripts + keys to panda /tmp ..."
ssh "$PANDA" "mkdir -p /tmp/pure_http_keys /tmp/web_auths /tmp/grok_batch"

# 脚本与依赖（Windows path 经 ssh 需在 Git Bash / WSL 下跑；PowerShell 用 scp 手动亦可）
for f in grok_pure_http_client.py grok_batch_yumail_gate.py grok_http_reliability_probe.py grok_playwright_common.py; do
  scp -q "$TNEXUS/scripts/$f" "$PANDA:/tmp/$f"
done

# OCR 探针图 + 已有 keys（若本机有）
if [[ -f "/tmp/grok_ocr_probe.png" ]]; then
  scp -q /tmp/grok_ocr_probe.png "$PANDA:/tmp/grok_ocr_probe.png" 2>/dev/null || true
fi
KEYS_FILE="$GROK_ROOT_LOCAL/reports/pure_http_keys/${EMAIL/@/_at_}.json"
if [[ -f "$KEYS_FILE" ]]; then
  scp -q "$KEYS_FILE" "$PANDA:/tmp/pure_http_keys/"
fi

# quota 列表（yumail 有额度）
QUOTA_JSON=$(ls -t "$GROK_ROOT_LOCAL/reports/pure_http_keys/quota_scan_"*.json 2>/dev/null | head -1 || true)
if [[ -n "${QUOTA_JSON}" && -f "${QUOTA_JSON}" ]]; then
  python3 - <<PY
import json, sys
from pathlib import Path
data = json.loads(Path(r"$QUOTA_JSON").read_text(encoding="utf-8"))
emails = [r["email"] for r in data.get("results", []) if r.get("ok") and (r.get("remainingQueries") or 0) >= 1]
Path("/tmp/yumail_quota_emails.txt").write_text("\n".join(emails[:$LIMIT]), encoding="utf-8")
print(f"emails={len(emails[:$LIMIT])}")
PY
  scp -q /tmp/yumail_quota_emails.txt "$PANDA:/tmp/yumail_quota_emails.txt"
fi

echo "[panda-udeal] run batch on panda (udeal egress id=110) ..."
ssh "$PANDA" "bash -s" <<'REMOTE'
set -euo pipefail
export GROK_LOCAL_PROXY="${GROK_LOCAL_PROXY:-http://127.0.0.1:18130}"
export GROK_KEYS_DIR=/tmp/pure_http_keys
export GROK_WEB_AUTHS=/tmp/web_auths
export PYTHONPATH=/tmp
cd /tmp

CRED_KEY=$(grep credentialEncryptionKey /opt/grok2api/config.yaml | head -1 | cut -d: -f2- | tr -d ' "')
export GROK_CREDENTIAL_KEY="$CRED_KEY"

python3 <<'PY'
import base64, json, os, sqlite3, subprocess, sys
from pathlib import Path

try:
    from cryptography.hazmat.primitives.ciphers.aead import AESGCM
except ImportError:
    subprocess.check_call([sys.executable, "-m", "pip", "install", "-q", "cryptography", "curl_cffi"])
    from cryptography.hazmat.primitives.ciphers.aead import AESGCM

def decrypt(enc_b64, key):
    pad = "=" * ((4 - len(enc_b64) % 4) % 4)
    raw = base64.b64decode(enc_b64 + pad)
    return AESGCM(key).decrypt(raw[:12], raw[12:], None).decode()

key = base64.b64decode(os.environ["GROK_CREDENTIAL_KEY"])
con = sqlite3.connect("file:/opt/grok2api/data/backend.db?mode=ro", uri=True)
row = con.execute("select encrypted_proxy_url from egress_nodes where id=110").fetchone()
con.close()
udeal = decrypt(row[0], key)
os.environ["GROK_UPSTREAM_PROXY"] = udeal
print("udeal", udeal[:30] + "...")
PY

# pip 依赖
python3 -m pip install -q curl_cffi cryptography 2>/dev/null || true

LIMIT="${LIMIT:-5}"
ROUNDS="${ROUNDS:-3}"
EMAIL="${EMAIL:-aclarkdc8c@yumail.co}"

if [[ -f /tmp/yumail_quota_emails.txt ]]; then
  python3 /tmp/grok_batch_yumail_gate.py \
    --emails-file /tmp/yumail_quota_emails.txt \
    --skip-extract \
    --signer python \
    --sleep 1.5 || true
fi

python3 /tmp/grok_http_reliability_probe.py \
  --email "$EMAIL" \
  --rounds "$ROUNDS" \
  --signer python \
  --json-out "/tmp/pure_http_keys/reliability_panda_${EMAIL/@/_at_}.json" || true

echo "=== batch reports ==="
ls -la /tmp/pure_http_keys/batch_gate/ 2>/dev/null || true
ls -la /tmp/pure_http_keys/reliability_*.json 2>/dev/null || true
REMOTE

echo "[panda-udeal] fetch reports ..."
scp -q "$PANDA:/tmp/pure_http_keys/batch_gate/*.json" "$GROK_ROOT_LOCAL/reports/pure_http_keys/batch_gate/" 2>/dev/null || true
scp -q "$PANDA:/tmp/pure_http_keys/reliability_panda_*.json" "$GROK_ROOT_LOCAL/reports/pure_http_keys/" 2>/dev/null || true
echo done
