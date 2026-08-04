#!/usr/bin/env python3
"""ETL grok2api SQLite backend.db → TNexus PostgreSQL grok_* tables.

See docs/39b-grok-schema.md. Skeleton — implement table COPY in G0.

Env:
  GROK_ETL_SOURCE   path to backend.db
  GROK_ETL_PG_DSN   postgres URL
  GROK_CREDENTIAL_KEY  optional, for post-ETL decrypt smoke
"""
from __future__ import annotations

import os
import sqlite3
import sys


def main() -> int:
    source = os.environ.get("GROK_ETL_SOURCE", "")
    dsn = os.environ.get("GROK_ETL_PG_DSN", "")
    if not source or not dsn:
        print("Set GROK_ETL_SOURCE and GROK_ETL_PG_DSN", file=sys.stderr)
        return 2
    if not os.path.isfile(source):
        print(f"Missing SQLite: {source}", file=sys.stderr)
        return 1

    con = sqlite3.connect(f"file:{source}?mode=ro", uri=True)
    cur = con.cursor()
    cur.execute("SELECT COUNT(*) FROM provider_accounts")
    count = cur.fetchone()[0]
    print(f"provider_accounts rows: {count}")
    # TODO(G0): COPY all 31 table families in dependency order via psycopg
    print("ETL skeleton OK — implement COPY per docs/39b §4")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
