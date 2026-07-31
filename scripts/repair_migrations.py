#!/usr/bin/env python3
"""Repair _sqlx_migrations checksums after local migration file edits."""
import hashlib
import os
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MIGRATIONS = ROOT / "migrations"


def checksum(path: Path) -> str:
    return hashlib.sha384(path.read_bytes()).hexdigest()


def main() -> None:
    db_url = os.environ.get("DATABASE_URL", "postgres://tnexus:tnexus@127.0.0.1:5432/tnexus")
    for sql in sorted(MIGRATIONS.glob("*.sql")):
        version = int(sql.name.split("_", 1)[0])
        cs = checksum(sql)
        sql_cmd = (
            f"UPDATE _sqlx_migrations SET checksum = decode('{cs}', 'hex') WHERE version = {version};"
        )
        subprocess.run(
            ["psql", db_url, "-c", sql_cmd],
            check=False,
            env={**os.environ, "PGPASSWORD": "tnexus"},
        )
        print(f"version {version}: {cs[:16]}…")


if __name__ == "__main__":
    main()
