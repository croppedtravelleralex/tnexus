#!/usr/bin/env python3
"""One-shot ETL: gptimage sqlite accounts.db → Postgres tnexus_accounts (migration 009)."""
from __future__ import annotations

import json
import os
import sqlite3
import sys

try:
    import psycopg2
    from psycopg2.extras import Json
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
        print(f"ACCOUNTS_DB not found: {ACCOUNTS_DB}", file=sys.stderr)
        return 1

    conn_pg = psycopg2.connect(DATABASE_URL)
    conn_sql = sqlite3.connect(ACCOUNTS_DB)
    cur = conn_sql.execute("SELECT access_token, data FROM accounts")
    rows = cur.fetchall()
    inserted = 0
    with conn_pg.cursor() as pg:
        for token, data_str in rows:
            token = (token or "").strip()
            if not token:
                continue
            try:
                data = json.loads(data_str or "{}")
            except json.JSONDecodeError:
                data = {}
            if not isinstance(data, dict):
                data = {}
            email = str(data.get("email") or "").strip().lower()
            if not email:
                email = f"import-{token[:8]}@local"
            data.pop("email", None)
            data.pop("access_token", None)
            data.pop("accessToken", None)
            pg.execute(
                """
                INSERT INTO tnexus_accounts (email, access_token, data, updated_at)
                VALUES (%s, %s, %s, now())
                ON CONFLICT (email) DO UPDATE
                SET access_token = EXCLUDED.access_token,
                    data = EXCLUDED.data,
                    updated_at = now()
                """,
                (email, token, Json(data)),
            )
            inserted += 1
        conn_pg.commit()
    conn_pg.close()
    conn_sql.close()
    print(f"upserted {inserted} accounts into tnexus_accounts")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
