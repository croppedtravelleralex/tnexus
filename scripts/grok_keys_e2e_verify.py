#!/usr/bin/env python3
"""对已提取的 pure_http keys 做纯 HTTP 端到端验证（conversations + chat PONG）。"""
from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import time
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

from grok_pure_http_client import KEYS_DIR, GrokPureHttpClient  # noqa: E402

PANDA = os.environ.get("PANDA_SSH", "panda")
PANDA_ENABLED_SCRIPT = os.environ.get(
    "PANDA_ENABLED_IDS_SCRIPT",
    "/root/TNexus/scripts/panda_list_grok_enabled_ids.py",
)
REPORT = ROOT / "reports" / "key_e2e_results.jsonl"


def panda_enabled_ids() -> list[int]:
    r = subprocess.run(
        ["ssh", PANDA, "python3", PANDA_ENABLED_SCRIPT],
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
        check=True,
    )
    return [int(x) for x in r.stdout.split() if x.strip().isdigit()]


def verify_delivery() -> dict:
    r = subprocess.run(
        ["ssh", PANDA, "bash", "/root/TNexus/scripts/verify_delivery.sh"],
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
        timeout=600,
    )
    return {"exit_code": r.returncode, "stdout_tail": "\n".join(r.stdout.splitlines()[-15:])}


def probe_account(account_id: int, keys_dir: Path, *, signer: str = "auto") -> dict:
    email = f"account_{account_id}@oldpool.local"
    keys_path = keys_dir / f"account_{account_id}.json"
    row: dict = {"account_id": account_id, "email": email}
    t0 = time.time()
    if not keys_path.exists():
        row.update({"ok": False, "error": "missing_keys_file"})
        return row
    try:
        keys = json.loads(keys_path.read_text(encoding="utf-8"))
        fp = (keys.get("fingerprint") or "").strip()
        row["fingerprint_len"] = len(fp)
        row["has_cf"] = keys.get("has_cf")
        if not keys.get("meta_b64") or len(fp) < 8:
            row.update({"ok": False, "error": "weak_key_no_fingerprint"})
            return row
        client = GrokPureHttpClient(
            keys,
            signer=signer,  # type: ignore[arg-type]
            upstream_proxy=os.environ.get("GROK_UPSTREAM_PROXY", ""),
        )
        r1 = client.request("GET", "/rest/app-chat/conversations")
        row["conversations_http"] = r1.status_code
        chat = client.chat_new("Reply with exactly: PONG")
        row["chat_ok"] = bool(chat.get("conversation_id"))
        row["response_id"] = chat.get("response_id")
        row["ok"] = row["conversations_http"] == 200 and row["chat_ok"]
    except Exception as exc:
        row.update({"ok": False, "error": f"{type(exc).__name__}:{exc}"})
    row["elapsed_s"] = round(time.time() - t0, 1)
    return row


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--account-ids", default="", help="comma-separated; default enabled on Panda")
    ap.add_argument("--from-panda-enabled", action="store_true", help="use enabled ids from Panda PG")
    ap.add_argument("--workers", type=int, default=4)
    ap.add_argument("--limit", type=int, default=0)
    ap.add_argument("--run-verify-delivery", action="store_true", help="also run verify_delivery.sh on Panda")
    ap.add_argument("--keys-dir", type=Path, default=KEYS_DIR)
    args = ap.parse_args()

    keys_dir = args.keys_dir

    if args.account_ids:
        ids = [int(x.strip()) for x in args.account_ids.split(",") if x.strip()]
    elif args.from_panda_enabled:
        ids = panda_enabled_ids()
    else:
        print("set --account-ids or --from-panda-enabled", file=sys.stderr)
        return 2
    if args.limit > 0:
        ids = ids[: args.limit]

    REPORT.parent.mkdir(parents=True, exist_ok=True)
    results: list[dict] = []
    workers = max(1, args.workers)
    with ThreadPoolExecutor(max_workers=workers) as pool:
        futs = {pool.submit(probe_account, aid, keys_dir): aid for aid in ids}
        for fut in as_completed(futs):
            row = fut.result()
            results.append(row)
            with REPORT.open("a", encoding="utf-8") as f:
                f.write(json.dumps(row, ensure_ascii=False) + "\n")
            mark = "OK" if row.get("ok") else "FAIL"
            print(f"{mark} id={row['account_id']} {row.get('error','')}", flush=True)

    ok = sum(1 for r in results if r.get("ok"))
    summary = {
        "n": len(results),
        "ok": ok,
        "fail": len(results) - ok,
        "report": str(REPORT),
    }
    if args.run_verify_delivery:
        summary["verify_delivery"] = verify_delivery()
    out_path = ROOT / "reports" / "key_e2e_summary.json"
    out_path.write_text(json.dumps(summary, ensure_ascii=False, indent=2), encoding="utf-8")
    print(json.dumps(summary, ensure_ascii=False))
    return 0 if ok == len(results) or len(results) == 0 else 1


if __name__ == "__main__":
    raise SystemExit(main())
