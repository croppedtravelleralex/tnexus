#!/usr/bin/env python3
"""逐个账号：Panda PG 解密 SSO → 本机 Playwright 提取 key → scp 到 Panda → sync enabled。

用法（本机 Windows/WSL，需 Playwright + 代理 7897）：
  python scripts/grok_extract_keys_sequential.py --from-panda --limit 20
  python scripts/grok_extract_keys_sequential.py --account-ids 86,304,100 --skip-sync
"""
from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

from extract_old_pool_session_keys import AUTHS_DIR, OUT_DIR, extract_for_account  # noqa: E402

PANDA = os.environ.get("PANDA_SSH", "panda")
PANDA_KEYS = os.environ.get("PANDA_KEYS", "/opt/tnexus/pure_http_keys")
LOG_PATH = ROOT / "reports" / "key_extract_progress.jsonl"


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
    sh(
        [
            "ssh",
            PANDA,
            "bash",
            "-lc",
            f"set -a; source /opt/tnexus/.env; set +a; "
            f"bash /root/TNexus/scripts/sync_grok_enabled_from_keys.sh "
            f"--keys-dir {PANDA_KEYS} --apply",
        ],
        check=False,
    )


def log_row(row: dict) -> None:
    LOG_PATH.parent.mkdir(parents=True, exist_ok=True)
    with LOG_PATH.open("a", encoding="utf-8") as f:
        f.write(json.dumps(row, ensure_ascii=False) + "\n")


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--account-ids", default="", help="comma-separated; default from --from-panda")
    ap.add_argument("--from-panda", action="store_true", help="fetch id list from Panda PG")
    ap.add_argument("--limit", type=int, default=0, help="max accounts (0=all)")
    ap.add_argument("--offset", type=int, default=0)
    ap.add_argument("--skip-existing", action="store_true", default=True)
    ap.add_argument("--no-skip-existing", action="store_false", dest="skip_existing")
    ap.add_argument("--skip-sync", action="store_true", help="do not run sync after each key")
    ap.add_argument("--headed", action="store_true")
    ap.add_argument("--sleep", type=float, default=2.0)
    args = ap.parse_args()

    if args.account_ids:
        ids = [int(x.strip()) for x in args.account_ids.split(",") if x.strip()]
    elif args.from_panda:
        ids = panda_account_ids(limit=args.limit, offset=args.offset)
    else:
        print("set --account-ids or --from-panda", file=sys.stderr)
        return 2

    ok = fail = skip = 0
    for i, aid in enumerate(ids):
        out_local = OUT_DIR / f"account_{aid}.json"
        if args.skip_existing and out_local.exists():
            try:
                keys = json.loads(out_local.read_text(encoding="utf-8"))
                fp = (keys.get("fingerprint") or "").strip()
                if keys.get("meta_b64") and len(fp) >= 8:
                    print(f"[{i+1}/{len(ids)}] skip {aid} (local key exists)")
                    skip += 1
                    continue
            except Exception:
                pass

        row: dict = {"account_id": aid, "i": i, "ts": time.strftime("%Y-%m-%dT%H:%M:%S")}
        t0 = time.time()
        try:
            auth = panda_fetch_sso(aid)
            AUTHS_DIR.mkdir(parents=True, exist_ok=True)
            (AUTHS_DIR / f"account_{aid}.json").write_text(
                json.dumps(auth, ensure_ascii=False, indent=2), encoding="utf-8"
            )
            out = extract_for_account(aid, headed=args.headed)
            keys = json.loads(out.read_text(encoding="utf-8"))
            row["has_cf"] = keys.get("has_cf")
            row["fingerprint_len"] = len(keys.get("fingerprint") or "")
            scp_key_to_panda(out)
            if not args.skip_sync:
                panda_sync_enabled()
            row["ok"] = True
            row["path"] = str(out)
            ok += 1
            print(
                f"[{i+1}/{len(ids)}] OK {aid} cf={keys.get('has_cf')} "
                f"fp={row['fingerprint_len']} ({time.time()-t0:.1f}s)"
            )
        except Exception as exc:
            row["ok"] = False
            row["error"] = f"{type(exc).__name__}:{exc}"
            fail += 1
            print(f"[{i+1}/{len(ids)}] FAIL {aid}: {exc}", file=sys.stderr)
        row["elapsed_s"] = round(time.time() - t0, 1)
        log_row(row)
        time.sleep(args.sleep)

    summary = {"ok": ok, "fail": fail, "skip": skip, "n": len(ids)}
    print(json.dumps(summary, ensure_ascii=False))
    return 0 if fail == 0 else 1


if __name__ == "__main__":
    raise SystemExit(main())
