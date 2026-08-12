#!/usr/bin/env python3
"""列出 Panda PG 中 grok_web 账号 id（stdout 每行一个）。"""
from __future__ import annotations

import subprocess
import sys
from urllib.parse import urlparse


def psql(sql: str) -> str:
    env = open("/opt/tnexus/.env", encoding="utf-8").read()
    dsn = next(l.split("=", 1)[1] for l in env.splitlines() if l.startswith("GROK_DATABASE_URL="))
    u = urlparse(dsn)
    user = u.username or "tnexus"
    db = (u.path or "/tnexus").lstrip("/") or "tnexus"
    return subprocess.check_output(
        ["docker", "exec", "panda-postgres-1", "psql", "-U", user, "-d", db, "-tAc", sql],
        text=True,
    )


def main() -> int:
    limit = int(sys.argv[1]) if len(sys.argv) > 1 else 0
    offset = int(sys.argv[2]) if len(sys.argv) > 2 else 0
    q = "SELECT id FROM grok_accounts WHERE provider='grok_web' ORDER BY id"
    if limit > 0:
        q += f" LIMIT {limit} OFFSET {offset}"
    out = psql(q)
    for line in out.splitlines():
        line = line.strip()
        if line.isdigit():
            print(line)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
