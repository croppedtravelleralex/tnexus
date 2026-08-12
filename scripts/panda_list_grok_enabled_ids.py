#!/usr/bin/env python3
"""列出 Panda PG 中 enabled 的 grok_web 账号 id（stdout 每行一个）。"""
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
    out = psql(
        "SELECT id FROM grok_accounts WHERE provider='grok_web' AND enabled ORDER BY id"
    )
    for line in out.splitlines():
        line = line.strip()
        if line.isdigit():
            print(line)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
