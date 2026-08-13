#!/usr/bin/env python3
import sqlite3
import sys

sys.path.insert(0, "/root/TNexus/scripts")
from grok_etl_sqlite_to_pg import build_plans, pg_column_types, _iter_source, _KEEP_RAW
import psycopg2

con = sqlite3.connect("file:/opt/grok2api/data/backend.db?mode=ro", uri=True)
pg = psycopg2.connect(
    "postgres://tnexus:914c7b5f0b459509cac9a474f9e8868e@127.0.0.1:5433/tnexus"
)
existing = pg_column_types(pg, "public")
plans = build_plans(con, pg, "public")
for p in plans:
    if p.target != "grok_accounts":
        continue
    cols = [c for c in p.columns if c in existing.get(p.target, {})]
    print("cols", len(cols), cols)
    row = next(_iter_source(con, p.source, cols, 1))
    print("row", len(row))
    break
