#!/usr/bin/env python3
"""One-off: align _sqlx_migrations checksum for 019 after file edit."""
import hashlib
import os
import subprocess
import sys

MIG = os.environ.get(
    "MIG_PATH", "/root/TNexus/migrations/019_grok_admin_alignment.sql"
)
DSN = os.environ.get(
    "DATABASE_URL",
    "postgres://tnexus:914c7b5f0b459509cac9a474f9e8868e@127.0.0.1:5433/tnexus",
)


def main() -> int:
    digest_hex = hashlib.sha384(open(MIG, "rb").read()).hexdigest()
    sql = (
        f"UPDATE _sqlx_migrations SET checksum = decode('{digest_hex}', 'hex') "
        "WHERE version = 19;"
    )
    env = os.environ.copy()
    if "PGPASSWORD" not in env and "@" in DSN:
        # postgres://user:pass@host:port/db
        userinfo = DSN.split("://", 1)[1].split("@", 1)[0]
        if ":" in userinfo:
            env["PGPASSWORD"] = userinfo.split(":", 1)[1]
    subprocess.run(
        [
            "psql",
            "-h",
            "127.0.0.1",
            "-p",
            "5433",
            "-U",
            "tnexus",
            "-d",
            "tnexus",
            "-c",
            sql,
        ],
        env=env,
        check=True,
    )
    print("updated migration 19 checksum")
    return 0


if __name__ == "__main__":
    sys.exit(main())
