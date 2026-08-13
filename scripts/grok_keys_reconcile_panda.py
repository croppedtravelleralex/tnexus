#!/usr/bin/env python3
"""对账本机与 Panda 的 pure_http_keys：把本机更优的 key 批量补传到 Panda。

单账号提取流程里 scp 偶发失败（exit 255）时，本地 key 已生成但 Panda 缺失，
而 --skip-existing 会永远跳过该账号。此脚本负责兜底补传。
"""
from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
LOCAL_KEYS = ROOT / "reports" / "pure_http_keys"
PANDA = os.environ.get("PANDA_SSH", "panda")
PANDA_KEYS = os.environ.get("PANDA_KEYS", "/opt/tnexus/pure_http_keys")
REMOTE_LISTER = "/root/grok_keys_list.py"


def local_map() -> dict[int, int]:
    out: dict[int, int] = {}
    for p in LOCAL_KEYS.glob("account_*.json"):
        m = re.fullmatch(r"account_(\d+)\.json", p.name)
        if not m:
            continue
        try:
            data = json.loads(p.read_text(encoding="utf-8"))
        except Exception:
            continue
        if not data.get("meta_b64"):
            continue
        out[int(m.group(1))] = len((data.get("fingerprint") or "").strip())
    return out


def panda_map() -> dict[int, int]:
    r = subprocess.run(
        ["ssh", PANDA, f"python3 {REMOTE_LISTER} {PANDA_KEYS}"],
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
        check=True,
    )
    out: dict[int, int] = {}
    for line in r.stdout.splitlines():
        parts = line.strip().split("\t")
        if len(parts) >= 2 and parts[0].isdigit():
            out[int(parts[0])] = int(parts[1])
    return out


def scp_chunk(paths: list[Path]) -> bool:
    cmd = ["scp", "-q", *[str(p) for p in paths], f"{PANDA}:{PANDA_KEYS}/"]
    for attempt in (1, 2):
        r = subprocess.run(cmd, capture_output=True, text=True, encoding="utf-8", errors="replace")
        if r.returncode == 0:
            return True
        print(f"  scp attempt {attempt} rc={r.returncode}: {r.stderr.strip()[:200]}", flush=True)
    return False


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--chunk", type=int, default=120)
    ap.add_argument("--apply", action="store_true")
    args = ap.parse_args()

    loc = local_map()
    pan = panda_map()
    print(f"local_keys={len(loc)} panda_keys={len(pan)}")

    # 需要补传：本机有 fp 而 Panda 缺失/更差
    todo = sorted(aid for aid, fp in loc.items() if fp > pan.get(aid, -1))
    print(f"to_upload={len(todo)}")
    if todo[:40]:
        print("ids_head=" + ",".join(str(a) for a in todo[:40]))
    if not todo or not args.apply:
        if todo and not args.apply:
            print("DRY-RUN: rerun with --apply")
        return 0

    ok = 0
    fail: list[int] = []
    for i in range(0, len(todo), args.chunk):
        part = todo[i : i + args.chunk]
        paths = [LOCAL_KEYS / f"account_{a}.json" for a in part]
        print(f">>> uploading {i + 1}..{i + len(part)} / {len(todo)}", flush=True)
        if scp_chunk(paths):
            ok += len(part)
        else:
            fail.extend(part)
    print(json.dumps({"uploaded": ok, "failed": len(fail), "failed_ids": fail[:50]}, ensure_ascii=False))
    return 0 if not fail else 1


if __name__ == "__main__":
    raise SystemExit(main())
