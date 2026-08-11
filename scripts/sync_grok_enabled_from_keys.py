#!/usr/bin/env python3
"""按 pure_http_keys 目录同步 grok_accounts.enabled（有 keys 才启用）。

用法（Panda 上 keys 已 scp 到 /opt/tnexus/pure_http_keys）：

  python scripts/sync_grok_enabled_from_keys.py \\
    --keys-dir /opt/tnexus/pure_http_keys \\
    --database-url "$GROK_DATABASE_URL" \\
    --dry-run

  # 仅启用有 keys 的 grok_web，其余禁用：
  python scripts/sync_grok_enabled_from_keys.py --keys-dir ... --database-url ... --apply
"""
from __future__ import annotations

import argparse
import json
import os
import re
import sys
from pathlib import Path

ACCOUNT_RE = re.compile(r"^account_(\d+)\.json$")


def list_key_account_ids(keys_dir: Path) -> set[int]:
    ids: set[int] = set()
    if not keys_dir.is_dir():
        raise SystemExit(f"keys dir not found: {keys_dir}")
    for path in keys_dir.glob("account_*.json"):
        m = ACCOUNT_RE.match(path.name)
        if not m:
            continue
        try:
            data = json.loads(path.read_text(encoding="utf-8"))
        except json.JSONDecodeError:
            continue
        fp = str(data.get("fingerprint") or "").strip()
        if fp:
            ids.add(int(m.group(1)))
    return ids


def main() -> None:
    p = argparse.ArgumentParser(description="Sync grok_accounts.enabled from pure_http_keys")
    p.add_argument("--keys-dir", default=os.environ.get("GROK_PURE_HTTP_KEYS_DIR", "reports/pure_http_keys"))
    p.add_argument("--database-url", default=os.environ.get("GROK_DATABASE_URL", ""))
    p.add_argument("--provider", default="grok_web")
    p.add_argument("--apply", action="store_true", help="write to DB (default dry-run)")
    p.add_argument("--dry-run", action="store_true", help="explicit dry-run")
    args = p.parse_args()
    dry_run = not args.apply or args.dry_run

    keys_dir = Path(args.keys_dir)
    key_ids = list_key_account_ids(keys_dir)
    print(f"keys_dir={keys_dir} accounts_with_keys={len(key_ids)} ids={sorted(key_ids)[:20]}{'…' if len(key_ids) > 20 else ''}")

    dsn = args.database_url.strip()
    if not dsn:
        print("GROK_DATABASE_URL / --database-url required for DB sync", file=sys.stderr)
        sys.exit(1)

    try:
        import psycopg
    except ImportError:
        print("pip install psycopg[binary]", file=sys.stderr)
        sys.exit(1)

    with psycopg.connect(dsn) as conn:
        with conn.cursor() as cur:
            cur.execute(
                "SELECT id, enabled FROM grok_accounts WHERE provider = %s ORDER BY id",
                (args.provider,),
            )
            rows = cur.fetchall()
            pool_ids = {r[0] for r in rows}
            enable_ids = sorted(key_ids & pool_ids)
            disable_ids = sorted(i for i, en in rows if i not in key_ids and en)
            missing_keys_in_pool = sorted(key_ids - pool_ids)

            print(f"pool_{args.provider}={len(pool_ids)} enable={len(enable_ids)} disable={len(disable_ids)}")
            if missing_keys_in_pool:
                print(f"warn: keys without pool row: {missing_keys_in_pool[:10]}")

            if dry_run:
                print("DRY-RUN: would ENABLE", enable_ids[:30], "…" if len(enable_ids) > 30 else "")
                print("DRY-RUN: would DISABLE", disable_ids[:30], "…" if len(disable_ids) > 30 else "")
                return

            if enable_ids:
                cur.execute(
                    "UPDATE grok_accounts SET enabled = true, updated_at = now() WHERE id = ANY(%s)",
                    (enable_ids,),
                )
            if disable_ids:
                cur.execute(
                    "UPDATE grok_accounts SET enabled = false, updated_at = now() WHERE id = ANY(%s)",
                    (disable_ids,),
                )
        conn.commit()
    print("applied OK")


if __name__ == "__main__":
    main()
