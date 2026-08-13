#!/usr/bin/env python3
"""诊断：对已有可用 fingerprint 的账号重跑提取，判断失败是账号失效还是全局被封。

安全性：先备份原 key，若重跑结果更差则自动还原，不会损坏已有可用 key。
"""
from __future__ import annotations

import argparse
import json
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
KEYS = ROOT / "reports" / "pure_http_keys"
BACKUP = ROOT / "reports" / "key_probe_backup"


def fp_len(path: Path) -> int:
    try:
        d = json.loads(path.read_text(encoding="utf-8"))
    except Exception:
        return -1
    return len((d.get("fingerprint") or "").strip())


def pick_good(n: int) -> list[int]:
    good = []
    for p in sorted(KEYS.glob("account_*.json")):
        stem = p.stem.replace("account_", "")
        if not stem.isdigit():
            continue
        if fp_len(p) >= 8:
            good.append(int(stem))
    if not good:
        return []
    # 取首、中、尾，覆盖新老账号段
    picks = [good[0], good[len(good) // 2], good[-1]]
    return list(dict.fromkeys(picks))[:n]


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--n", type=int, default=3)
    ap.add_argument("--ids", default="")
    args = ap.parse_args()

    ids = [int(x) for x in args.ids.split(",") if x.strip()] if args.ids else pick_good(args.n)
    if not ids:
        print("no known-good accounts found")
        return 1

    BACKUP.mkdir(parents=True, exist_ok=True)
    before = {}
    for aid in ids:
        src = KEYS / f"account_{aid}.json"
        before[aid] = fp_len(src)
        shutil.copy2(src, BACKUP / src.name)
    print(f"probing known-good accounts: {before}")

    mtime_before = {aid: (KEYS / f"account_{aid}.json").stat().st_mtime for aid in ids}

    subprocess.call(
        [
            sys.executable,
            str(ROOT / "scripts" / "grok_extract_keys_sequential.py"),
            "--account-ids",
            ",".join(str(a) for a in ids),
            "--no-skip-existing",
            "--skip-sync",
            "--workers",
            "1",
        ]
    )

    # 提取失败时文件不会被改写，因此必须用 mtime 判断本次是否真的重新提取成功，
    # 不能只看 fingerprint 长度（那仍是旧值）。
    verdict = {}
    reextracted = 0
    for aid in ids:
        src = KEYS / f"account_{aid}.json"
        rewritten = src.stat().st_mtime > mtime_before[aid]
        after = fp_len(src)
        verdict[aid] = {"before": before[aid], "after": after, "reextracted": rewritten}
        if rewritten:
            reextracted += 1
            if after < before[aid]:
                shutil.copy2(BACKUP / src.name, src)
                verdict[aid]["restored"] = True

    print(json.dumps(verdict, ensure_ascii=False, indent=2))
    print(
        f"\nVERDICT: {reextracted}/{len(ids)} known-good accounts re-extracted successfully → "
        + (
            "extraction pipeline healthy; residual failures are dead accounts"
            if reextracted
            else "GLOBAL BLOCK: grok.com serves an app-less page to this IP/proxy; "
            "retrying other accounts is futile until it clears"
        )
    )
    return 0 if reextracted else 2


if __name__ == "__main__":
    raise SystemExit(main())
