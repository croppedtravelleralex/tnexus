#!/usr/bin/env python3
"""Print udeal egress proxy URL from Panda SQLite (stdout only)."""
from __future__ import annotations

import base64
import subprocess
import sys

REMOTE = r"""
import base64, sqlite3
from cryptography.hazmat.primitives.ciphers.aead import AESGCM
import re
cfg = open('/opt/grok2api/config.yaml').read()
m = re.search(r'credentialEncryptionKey:\s*\"([^\"]+)\"', cfg)
key = base64.b64decode(m.group(1))
def dec(s):
    pad = '=' * ((4-len(s)%4)%4)
    raw = base64.b64decode(s+pad)
    return AESGCM(key).decrypt(raw[:12], raw[12:], None).decode()
con = sqlite3.connect('file:/opt/grok2api/data/backend.db?mode=ro', uri=True)
row = con.execute('select encrypted_proxy_url from egress_nodes where id=110').fetchone()
print(dec(row[0]))
"""

def main() -> int:
    host = sys.argv[1] if len(sys.argv) > 1 else "panda"
    b64 = base64.b64encode(REMOTE.encode()).decode()
    cmd = f"python3 -c \"import base64; exec(base64.b64decode('{b64}').decode())\""
    proc = subprocess.run(
        ["ssh", "-o", "BatchMode=yes", host, cmd],
        capture_output=True,
        text=True,
        timeout=30,
    )
    if proc.returncode != 0:
        print(proc.stderr or proc.stdout, file=sys.stderr)
        return proc.returncode
    print(proc.stdout.strip())
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
