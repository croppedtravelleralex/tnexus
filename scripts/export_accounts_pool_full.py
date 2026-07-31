#!/usr/bin/env python3
"""Export full account payloads from gptimage sqlite for TNexus import.

Usage (on Panda):
  ACCOUNTS_DB=/root/gptimage/data/accounts.db \
  OUT_PATH=/tmp/accounts_pool_full.json \
  python3 scripts/export_accounts_pool_full.py

Env:
  ACCOUNTS_DB   sqlite path (default /app/data/accounts.db)
  OUT_PATH      output JSON array
  LIMIT         max rows (default 0 = all)
  INCLUDE_ABNORMAL  1 to include 异常/禁用 (default 0)
"""
from __future__ import annotations

import json
import os
import sqlite3
import sys


def main() -> int:
    db = os.environ.get("ACCOUNTS_DB", "/app/data/accounts.db")
    out = os.environ.get("OUT_PATH", "/tmp/accounts_pool_full.json")
    limit = int(os.environ.get("LIMIT", "0") or "0")
    include_abnormal = str(os.environ.get("INCLUDE_ABNORMAL", "0")).strip() in {"1", "true", "yes"}

    conn = sqlite3.connect(db)
    rows = list(conn.execute("select id, access_token, data from accounts"))
    conn.close()

    items: list[dict] = []
    for _id, token, data in rows:
        try:
            d = json.loads(data) if isinstance(data, str) else (data or {})
        except Exception:
            d = {}
        if not isinstance(d, dict):
            d = {}
        if token and not d.get("access_token"):
            d["access_token"] = token
        email = str(d.get("email") or "").strip()
        if not email or not str(d.get("access_token") or "").strip():
            continue
        status = str(d.get("status") or "")
        if not include_abnormal and status in {"禁用", "异常"}:
            continue
        items.append(d)

    items.sort(key=lambda row: str(row.get("email") or "").lower())
    if limit > 0:
        items = items[:limit]

    if not items:
        print("NO_CANDIDATES", file=sys.stderr)
        return 2

    with open(out, "w", encoding="utf-8") as f:
        json.dump(items, f, ensure_ascii=False, indent=2)

    print(
        json.dumps(
            {
                "ok": True,
                "out": out,
                "count": len(items),
                "sample_emails": [str(a.get("email") or "") for a in items[:5]],
            },
            ensure_ascii=False,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
