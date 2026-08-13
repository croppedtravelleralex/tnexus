#!/usr/bin/env python3
"""在 panda-account-ops 容器内刷新已过期的 access_token，并同步到 Panda gateway。

用法（Panda）：
  docker exec panda-account-ops-1 python3 /tmp/_panda_refresh_expired.py --dry-run
  docker exec panda-account-ops-1 python3 /tmp/_panda_refresh_expired.py --apply
"""
from __future__ import annotations

import argparse
import base64
import json
import sys
import time

sys.path.insert(0, "/app")

from services.account_service import account_service  # noqa: E402
from services.account_refresh_all_service import account_refresh_all_service  # noqa: E402


def jwt_exp(tok: str) -> int | None:
    try:
        p = tok.split(".")[1]
        p += "=" * (-len(p) % 4)
        return json.loads(base64.urlsafe_b64decode(p)).get("exp")
    except Exception:
        return None


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--apply", action="store_true")
    ap.add_argument("--dry-run", action="store_true")
    # 剩余有效期低于该秒数也一并刷新（默认 24h）
    ap.add_argument("--min-ttl", type=int, default=86400)
    ap.add_argument("--include-errored", action="store_true",
                    help="也刷新此前 last_token_refresh_error 非空的账号")
    args = ap.parse_args()

    now = int(time.time())
    tokens = account_service.list_tokens()
    expired, soon, healthy, skipped = [], [], [], []

    for token in tokens:
        acct = account_service.get_account(token) or {}
        err = str(acct.get("last_token_refresh_error") or "").strip()
        if err and not args.include_errored:
            skipped.append((acct.get("email"), err[:60]))
            continue
        exp = jwt_exp(acct.get("access_token") or "")
        if exp is None:
            expired.append(token)
        elif exp < now:
            expired.append(token)
        elif exp - now < args.min_ttl:
            soon.append(token)
        else:
            healthy.append(token)

    print(f"total={len(tokens)} expired={len(expired)} expiring_soon={len(soon)} "
          f"healthy={len(healthy)} skipped_errored={len(skipped)}", flush=True)
    for email, err in skipped[:10]:
        print(f"  skip {email}: {err}", flush=True)

    targets = expired + soon
    if not targets:
        print("nothing to refresh", flush=True)
        return 0

    if not args.apply:
        print(f"DRY-RUN would refresh {len(targets)} accounts", flush=True)
        return 0

    result = account_service.refresh_accounts(targets, None, False, False)
    errs = result.get("errors") or []
    print(f"refreshed={result.get('refreshed', 0)} errors={len(errs)}", flush=True)
    for e in errs[:10]:
        print("  err:", e, flush=True)

    sync = account_refresh_all_service.sync_last_refreshed_accounts_to_panda()
    print(f"panda_sync={sync}", flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
