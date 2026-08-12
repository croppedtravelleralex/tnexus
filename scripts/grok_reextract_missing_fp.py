#!/usr/bin/env python3
"""重提 fingerprint 为空的账号。

早期批次很多账号提取成功但 fingerprint 为空（fp=0），这类 key 过不了
sync_grok_enabled_from_keys.sh 的筛选，等于白提。本脚本挑出这些账号，
用 --no-skip-existing 强制重提。

  python scripts/grok_reextract_missing_fp.py --dry-run
  python scripts/grok_reextract_missing_fp.py --apply --workers 6 --chunk 40
"""
from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SEQ = ROOT / "scripts" / "grok_extract_keys_sequential.py"
PROGRESS = ROOT / "reports" / "key_extract_progress.jsonl"


def missing_fp_ids(min_fp: int = 8) -> list[int]:
    latest: dict[int, dict] = {}
    for line in PROGRESS.read_text(encoding="utf-8", errors="replace").splitlines():
        try:
            row = json.loads(line)
        except json.JSONDecodeError:
            continue
        aid = row.get("account_id")
        if isinstance(aid, int):
            latest[aid] = row
    return sorted(a for a, r in latest.items() if r.get("fingerprint_len", 0) < min_fp)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--apply", action="store_true")
    ap.add_argument("--workers", type=int, default=6)
    ap.add_argument("--chunk", type=int, default=40, help="accounts per subprocess batch")
    ap.add_argument("--min-fp", type=int, default=8)
    args = ap.parse_args()

    ids = missing_fp_ids(args.min_fp)
    print(f"missing_fingerprint={len(ids)}", flush=True)
    if not ids:
        return 0
    if not args.apply:
        print("DRY-RUN ids:", ",".join(map(str, ids)), flush=True)
        return 0

    for i in range(0, len(ids), args.chunk):
        batch = ids[i : i + args.chunk]
        print(f"\n>>> re-extract {i + 1}..{i + len(batch)} of {len(ids)}", flush=True)
        subprocess.call([
            sys.executable, str(SEQ),
            "--account-ids", ",".join(map(str, batch)),
            "--no-skip-existing",
            "--workers", str(args.workers),
            "--sleep", "0.3",
            "--skip-sync",
        ])

    still = missing_fp_ids(args.min_fp)
    print(json.dumps({
        "attempted": len(ids),
        "recovered": len(ids) - len(still),
        "still_missing": len(still),
    }, ensure_ascii=False), flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
