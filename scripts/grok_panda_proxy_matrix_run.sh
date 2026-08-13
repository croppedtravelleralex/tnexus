#!/usr/bin/env bash
# Panda 四路代理 gate（Python 探针；Rust 二进制需 Linux CI 产物）
set -euo pipefail
export GROK_LOCAL_PROXY="${GROK_LOCAL_PROXY:-http://127.0.0.1:18130}"
export PYTHONPATH=/tmp
cd /tmp

CRED_KEY=$(grep credentialEncryptionKey /opt/grok2api/config.yaml | head -1 | cut -d: -f2- | tr -d ' "')
export GROK_CREDENTIAL_KEY="$CRED_KEY"

python3 <<'PY'
import base64, json, os, sqlite3, sys
from pathlib import Path

sys.path.insert(0, "/tmp")

try:
    from cryptography.hazmat.primitives.ciphers.aead import AESGCM
except ImportError:
    import subprocess
    subprocess.check_call([sys.executable, "-m", "pip", "install", "-q", "cryptography"])
    from cryptography.hazmat.primitives.ciphers.aead import AESGCM


def decrypt(enc_b64: str, key: bytes) -> str | None:
    if not enc_b64:
        return None
    pad = "=" * ((4 - len(enc_b64) % 4) % 4)
    raw = base64.b64decode(enc_b64 + pad)
    nonce, ct = raw[:12], raw[12:]
    return AESGCM(key).decrypt(nonce, ct, None).decode()


key = base64.b64decode(os.environ["GROK_CREDENTIAL_KEY"])
con = sqlite3.connect("file:/opt/grok2api/data/backend.db?mode=ro", uri=True)
rows = con.execute("select id, encrypted_proxy_url from egress_nodes where enabled=1").fetchall()
con.close()
proxies = {}
for nid, enc in rows:
    url = decrypt(enc, key)
    if url:
        proxies[nid] = url

# 110=udeal 惯例；另取前两个 webshare 风格节点作 DC/住宅代表
udeal = proxies.get(110) or next(iter(proxies.values()), "")
others = [v for k, v in sorted(proxies.items()) if k != 110][:2]

from grok_pure_http_client import run_gate

cases = [("direct", "")]
if udeal:
    cases.append(("udeal", udeal))
if others:
    cases.append(("webshare_a", others[0]))
if len(others) > 1:
    cases.append(("webshare_b", others[1]))

summary = []
for label, upstream in cases:
    if upstream:
        os.environ["GROK_UPSTREAM_PROXY"] = upstream
    else:
        os.environ.pop("GROK_UPSTREAM_PROXY", None)
    try:
        r = run_gate(
            "nancybaker2jyy@yumail.co",
            extract=False,
            headed=False,
            signer="python",
            image_path=Path("/tmp/grok_ocr_probe.png"),
            keys_path=Path("/tmp/nancy_keys.json"),
        )
        r["proxy_label"] = label
        r["upstream_masked"] = upstream[:20] + "..." if upstream else ""
        out = Path(f"/tmp/grok_proxy_matrix/gate_{label}.json")
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_text(json.dumps(r, ensure_ascii=False, indent=2), encoding="utf-8")
        summary.append({
            "label": label,
            "ok": r.get("ok"),
            "upload_ok": r.get("upload_ok"),
            "followup_ok": r.get("followup_ok"),
            "ocr_ok": r.get("ocr_ok"),
        })
        print(json.dumps(summary[-1], ensure_ascii=False))
    except Exception as e:
        summary.append({"label": label, "ok": False, "error": str(e)})
        print(json.dumps(summary[-1], ensure_ascii=False))

Path("/tmp/grok_proxy_matrix/summary.json").write_text(
    json.dumps(summary, ensure_ascii=False, indent=2), encoding="utf-8"
)
PY
