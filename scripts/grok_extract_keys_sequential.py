#!/usr/bin/env python3
"""逐个账号：Panda PG 解密 SSO → 本机 Playwright 提取 key → scp 到 Panda → sync enabled。

用法（本机 Windows/WSL，需 Playwright + 代理 7897）：
  python scripts/grok_extract_keys_sequential.py --from-panda --limit 20 --workers 4
  python scripts/grok_extract_keys_sequential.py --account-ids 86,304,100 --skip-sync
"""
from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import threading
import time
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

from extract_old_pool_session_keys import AUTHS_DIR, OUT_DIR, extract_for_account  # noqa: E402

PANDA = os.environ.get("PANDA_SSH", "panda")
PANDA_KEYS = os.environ.get("PANDA_KEYS", "/opt/tnexus/pure_http_keys")
LOG_PATH = ROOT / "reports" / "key_extract_progress.jsonl"
_LOG_LOCK = threading.Lock()
_SYNC_LOCK = threading.Lock()


def sh(cmd: list[str], *, check: bool = True) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        cmd,
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
        check=check,
    )


def panda_account_ids(limit: int = 0, offset: int = 0) -> list[int]:
    cmd = ["ssh", PANDA, "python3", "/root/TNexus/scripts/panda_list_grok_ids.py"]
    if limit > 0:
        cmd.extend([str(limit), str(offset)])
    r = subprocess.run(cmd, capture_output=True, text=True, encoding="utf-8", errors="replace", check=True)
    return [int(x) for x in r.stdout.split() if x.strip().isdigit()]


def panda_fetch_sso(account_id: int) -> dict:
    r = subprocess.run(
        ["ssh", PANDA, "python3", "/root/TNexus/scripts/panda_fetch_sso.py", str(account_id)],
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
        check=True,
    )
    data = json.loads(r.stdout.strip().splitlines()[-1])
    if data.get("error"):
        raise RuntimeError(data["error"])
    if not data.get("sso"):
        raise RuntimeError(f"account {account_id}: empty sso")
    return data


def scp_key_to_panda(local_path: Path) -> None:
    sh(["scp", str(local_path), f"{PANDA}:{PANDA_KEYS}/"], check=True)


def panda_sync_enabled() -> None:
    # ssh 会把多余 argv 用空格拼成一条远端命令行，因此整段必须自带引号，
    # 否则 `source .env` 与脚本会落在不同的 shell 里，DSN 传不进去。
    remote = (
        "bash -lc 'set -a; source /opt/tnexus/.env; set +a; "
        "bash /root/TNexus/scripts/sync_grok_enabled_from_keys.sh "
        f"--keys-dir {PANDA_KEYS} --apply'"
    )
    with _SYNC_LOCK:
        sh(["ssh", PANDA, remote], check=False)


def log_row(row: dict) -> None:
    LOG_PATH.parent.mkdir(parents=True, exist_ok=True)
    with _LOG_LOCK:
        with LOG_PATH.open("a", encoding="utf-8") as f:
            f.write(json.dumps(row, ensure_ascii=False) + "\n")


def should_skip(aid: int, skip_existing: bool) -> bool:
    if not skip_existing:
        return False
    out_local = OUT_DIR / f"account_{aid}.json"
    if not out_local.exists():
        return False
    try:
        keys = json.loads(out_local.read_text(encoding="utf-8"))
        fp = (keys.get("fingerprint") or "").strip()
        return bool(keys.get("meta_b64") and len(fp) >= 8)
    except Exception:
        return False


def process_one(
    aid: int,
    i: int,
    total: int,
    *,
    headed: bool,
    skip_sync_per_account: bool,
) -> dict:
    row: dict = {"account_id": aid, "i": i, "ts": time.strftime("%Y-%m-%dT%H:%M:%S")}
    t0 = time.time()
    try:
        auth = panda_fetch_sso(aid)
        AUTHS_DIR.mkdir(parents=True, exist_ok=True)
        (AUTHS_DIR / f"account_{aid}.json").write_text(
            json.dumps(auth, ensure_ascii=False, indent=2), encoding="utf-8"
        )
        out = extract_for_account(aid, headed=headed)
        keys = json.loads(out.read_text(encoding="utf-8"))
        row["has_cf"] = keys.get("has_cf")
        row["fingerprint_len"] = len(keys.get("fingerprint") or "")
        scp_key_to_panda(out)
        if skip_sync_per_account:
            panda_sync_enabled()
        row["ok"] = True
        row["path"] = str(out)
        print(
            f"[{i+1}/{total}] OK {aid} cf={keys.get('has_cf')} fp={row['fingerprint_len']} "
            f"({time.time()-t0:.1f}s)",
            flush=True,
        )
    except Exception as exc:
        row["ok"] = False
        row["error"] = f"{type(exc).__name__}:{exc}"
        print(f"[{i+1}/{total}] FAIL {aid}: {exc}", file=sys.stderr, flush=True)
    row["elapsed_s"] = round(time.time() - t0, 1)
    log_row(row)
    return row


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--account-ids", default="", help="comma-separated; default from --from-panda")
    ap.add_argument("--from-panda", action="store_true", help="fetch id list from Panda PG")
    ap.add_argument("--limit", type=int, default=0, help="max accounts (0=all)")
    ap.add_argument("--offset", type=int, default=0)
    ap.add_argument("--workers", type=int, default=8, help="parallel Playwright workers (default 8, max 12)")
    ap.add_argument("--skip-existing", action="store_true", default=True)
    ap.add_argument("--no-skip-existing", action="store_false", dest="skip_existing")
    ap.add_argument("--skip-sync", action="store_true", help="never sync during run")
    ap.add_argument("--headed", action="store_true")
    ap.add_argument("--sleep", type=float, default=0.5, help="delay between task submits")
    args = ap.parse_args()

    if args.account_ids:
        ids = [int(x.strip()) for x in args.account_ids.split(",") if x.strip()]
    elif args.from_panda:
        ids = panda_account_ids(limit=args.limit, offset=args.offset)
    else:
        print("set --account-ids or --from-panda", file=sys.stderr)
        return 2

    workers = max(1, min(args.workers, 12))
    # 并发时只在每账号后 sync 会打爆 Panda；默认批末一次 sync
    sync_per_account = not args.skip_sync and workers == 1

    pending: list[tuple[int, int]] = []
    skip = 0
    for i, aid in enumerate(ids):
        if should_skip(aid, args.skip_existing):
            print(f"[{i+1}/{len(ids)}] skip {aid} (local key exists)", flush=True)
            skip += 1
            continue
        pending.append((aid, i))

    ok = fail = 0
    if not pending:
        print(json.dumps({"ok": 0, "fail": 0, "skip": skip, "n": len(ids)}, ensure_ascii=False))
        return 0

    if workers == 1:
        for aid, i in pending:
            row = process_one(aid, i, len(ids), headed=args.headed, skip_sync_per_account=sync_per_account)
            ok += int(bool(row.get("ok")))
            fail += int(not row.get("ok"))
            time.sleep(args.sleep)
    else:
        with ThreadPoolExecutor(max_workers=workers) as pool:
            futs = []
            for aid, i in pending:
                futs.append(
                    pool.submit(
                        process_one,
                        aid,
                        i,
                        len(ids),
                        headed=args.headed,
                        skip_sync_per_account=False,
                    )
                )
                time.sleep(args.sleep)
            for fut in as_completed(futs):
                row = fut.result()
                ok += int(bool(row.get("ok")))
                fail += int(not row.get("ok"))

    if not args.skip_sync:
        print(">>> batch sync enabled on Panda", flush=True)
        panda_sync_enabled()

    summary = {"ok": ok, "fail": fail, "skip": skip, "n": len(ids), "workers": workers}
    print(json.dumps(summary, ensure_ascii=False))
    return 0 if fail == 0 else 1


if __name__ == "__main__":
    raise SystemExit(main())
