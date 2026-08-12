#!/usr/bin/env python3
"""连续分批 Playwright 提取 key，直到 Panda 号池扫完或达到 --max-batches。"""
from __future__ import annotations

import argparse
import json
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SEQ = ROOT / "scripts" / "grok_extract_keys_sequential.py"


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--start-offset", type=int, default=83)
    ap.add_argument("--batch-size", type=int, default=50)
    ap.add_argument("--max-batches", type=int, default=0, help="0 = until pool end")
    ap.add_argument("--sleep-between-batches", type=float, default=10.0)
    ap.add_argument("--sync-each-batch", action="store_true", default=True)
    args = ap.parse_args()

    offset = args.start_offset
    batches = 0
    while True:
        if args.max_batches and batches >= args.max_batches:
            break
        print(f"\n>>> batch {batches + 1} offset={offset} limit={args.batch_size}", flush=True)
        cmd = [
            sys.executable,
            str(SEQ),
            "--from-panda",
            "--limit",
            str(args.batch_size),
            "--offset",
            str(offset),
            "--sleep",
            "3",
        ]
        if not args.sync_each_batch:
            cmd.append("--skip-sync")
        rc = subprocess.call(cmd)
        batches += 1
        # 若返回非 0 且本批无进展，停止
        if rc != 0:
            print(f"batch failed rc={rc}, stopping", flush=True)
            break
        offset += args.batch_size
        if offset >= 698:
            print("reached pool end (~698 accounts)", flush=True)
            break
        time.sleep(args.sleep_between_batches)

    print(json.dumps({"batches": batches, "next_offset": offset}, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
