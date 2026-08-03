#!/usr/bin/env python3
"""Compare sqlite accounts.db row count vs Postgres tnexus_accounts (post-ETL sanity)."""
from __future__ import annotations

import os
import sqlite3
import sys

try:
    import psycopg2
except ImportError:
    print("pip install psycopg2-binary", file=sys.stderr)
    raise

ACCOUNTS_DB = os.environ.get("ACCOUNTS_DB", "data/gptimage/accounts.db")
DATABASE_URL = os.environ.get("DATABASE_URL", "")


def main() -> int:
    if not DATABASE_URL:
        print("DATABASE_URL required", file=sys.stderr)
        return 1
    if not os.path.isfile(ACCOUNTS_DB):
        print(f"sqlite not found: {ACCOUNTS_DB} (skip sqlite side)", file=sys.stderr)
        sqlite_count = None
    else:
        conn = sqlite3.connect(ACCOUNTS_DB)
        sqlite_count = conn.execute("SELECT COUNT(*) FROM accounts").fetchone()[0]
        conn.close()

    conn_pg = psycopg2.connect(DATABASE_URL)
    with conn_pg.cursor() as cur:
        cur.execute("SELECT COUNT(*) FROM tnexus_accounts")
        pg_count = cur.fetchone()[0]
    conn_pg.close()

    print(f"postgres tnexus_accounts: {pg_count}")
    if sqlite_count is not None:
        print(f"sqlite accounts: {sqlite_count}")
        delta = pg_count - sqlite_count
        if delta == 0:
            print("ok: counts match")
            return 0
        print(f"warning: delta={delta} (postgres - sqlite)")
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
