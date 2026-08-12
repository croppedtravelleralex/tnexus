#!/usr/bin/env python3
"""连续分批 Playwright 提取 key，直到 Panda 号池扫完或达到 --max-batches。"""
from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SEQ = ROOT / "scripts" / "grok_extract_keys_sequential.py"


def panda_sync_enabled() -> None:
    # ssh 会把多余 argv 用空格拼成一条远端命令行，因此整段必须自带引号，
    # 否则 `source .env` 与脚本会落在不同的 shell 里，DSN 传不进去。
    remote = (
        "bash -lc 'set -a; source /opt/tnexus/.env; set +a; "
        "bash /root/TNexus/scripts/sync_grok_enabled_from_keys.sh "
        "--keys-dir /opt/tnexus/pure_http_keys --apply'"
    )
    subprocess.call(["ssh", os.environ.get("PANDA_SSH", "panda"), remote])


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--start-offset", type=int, default=83)
    ap.add_argument("--batch-size", type=int, default=50)
    ap.add_argument("--max-batches", type=int, default=0, help="0 = until pool end")
    ap.add_argument("--workers", type=int, default=8, help="parallel Playwright workers per batch")
    ap.add_argument("--sleep-between-batches", type=float, default=5.0)
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
            "--workers",
            str(args.workers),
            "--sleep",
            "0.3",
            "--skip-sync",
        ]
        rc = subprocess.call(cmd)
        batches += 1
        if args.sync_each_batch:
            panda_sync_enabled()
        if rc != 0:
            print(f"batch rc={rc} (partial failures ok), continuing", flush=True)
        offset += args.batch_size
        if offset >= 698:
            print("reached pool end (~698 accounts)", flush=True)
            break
        time.sleep(args.sleep_between_batches)

    print(json.dumps({"batches": batches, "next_offset": offset}, ensure_ascii=False))
    panda_sync_enabled()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
