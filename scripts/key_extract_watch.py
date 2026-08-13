#!/usr/bin/env python3
"""快速查看当前提取批次进度（不解析全量 jsonl）。"""
from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_LOG = ROOT / "reports" / "key_extract_batch.log"


def main() -> int:
    tail_n = int(sys.argv[1]) if len(sys.argv) > 1 else 12
    log = Path(sys.argv[2]) if len(sys.argv) > 2 else DEFAULT_LOG
    lines = log.read_text(encoding="utf-8", errors="replace").splitlines()
    batch = ""
    for line in lines:
        if line.startswith(">>> batch "):
            batch = line
    done = [l for l in lines if re.match(r"^\[\d+/\d+\] (OK|FAIL|skip)", l)]
    # 只统计当前批次之后的行
    if batch:
        idx = len(lines) - 1 - lines[::-1].index(batch)
        cur = [l for l in lines[idx:] if re.match(r"^\[\d+/\d+\] (OK|FAIL|skip)", l)]
    else:
        cur = done
    ok = sum(1 for l in cur if "] OK " in l)
    fail = sum(1 for l in cur if "] FAIL " in l)
    skip = sum(1 for l in cur if "] skip " in l)
    fp0 = sum(1 for l in cur if re.search(r"fp=0\b", l))
    print(f"current: {batch}")
    print(f"batch_done={len(cur)} ok={ok} fail={fail} skip={skip} fp0={fp0}")
    print(f"total_log_lines={len(lines)}")
    print("--- tail ---")
    for l in lines[-tail_n:]:
        print(l)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
