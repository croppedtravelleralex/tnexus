#!/usr/bin/env python3
"""在 Panda 上解密单个 grok_web 账号 SSO（stdout JSON）。"""
from __future__ import annotations

import base64
import json
import re
import subprocess
import sys
from urllib.parse import urlparse


def psql(sql: str, *, field_sep: str | None = None) -> str:
    """经 panda-postgres 容器内 socket 查询（避免 127.0.0.1:5433 在容器内不可达）。"""
    env = open("/opt/tnexus/.env", encoding="utf-8").read()
    dsn = next(l.split("=", 1)[1] for l in env.splitlines() if l.startswith("GROK_DATABASE_URL="))
    u = urlparse(dsn)
    user = u.username or "tnexus"
    db = (u.path or "/tnexus").lstrip("/") or "tnexus"
    cmd = ["docker", "exec", "panda-postgres-1", "psql", "-U", user, "-d", db]
    if field_sep:
        cmd.extend(["-tA", "-F", field_sep, "-c", sql])
    else:
        cmd.extend(["-tAc", sql])
    return subprocess.check_output(cmd, text=True)


def main() -> int:
    if len(sys.argv) < 2:
        print("usage: panda_fetch_sso.py <account_id>", file=sys.stderr)
        return 2
    aid = int(sys.argv[1])
    cfg = open("/opt/grok2api/config.yaml", encoding="utf-8").read()
    m = re.search(r'credentialEncryptionKey:\s*"([^"]+)"', cfg)
    if not m:
        print(json.dumps({"account_id": aid, "error": "no credentialEncryptionKey"}))
        return 1
    key = base64.b64decode(m.group(1))
    row = psql(
        "SELECT ga.id::text, ga.identity_key, gc.encrypted_primary "
        "FROM grok_accounts ga JOIN grok_credentials gc ON gc.account_id=ga.id "
        f"WHERE ga.id={aid} AND ga.provider='grok_web' "
        "AND gc.encrypted_primary IS NOT NULL AND gc.encrypted_primary <> ''",
        field_sep="|",
    ).strip()
    if not row:
        print(json.dumps({"account_id": aid, "error": "no credential"}))
        return 0
    parts = row.split("|", 2)
    if len(parts) < 3:
        print(json.dumps({"account_id": aid, "error": f"bad row: {row[:80]}"}))
        return 1
    _, identity, enc = parts
    from cryptography.hazmat.primitives.ciphers.aead import AESGCM

    raw = base64.b64decode(enc)
    sso = AESGCM(key).decrypt(raw[:12], raw[12:], None).decode("utf-8")
    print(json.dumps({"account_id": aid, "identity_key": identity, "sso": sso}, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
