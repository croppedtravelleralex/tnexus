#!/usr/bin/env python3
"""汇总 Grok 号池 session key 提取结果 → reports/key_extract_summary.json。

数据来源：
  - reports/pool_ids.txt            全池 grok_web 账号 id（Panda PG）
  - reports/pure_http_keys/*.json   本机提取产物（fingerprint 质量以此为准）
  - reports/key_extract_progress.jsonl  每次尝试的成败与错误
  - Panda: keys 目录清单 + grok_accounts.enabled
"""
from __future__ import annotations

import argparse
import collections
import json
import os
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

from key_extract_stats import err_bucket, load_progress, scan_local_keys  # noqa: E402

PANDA = os.environ.get("PANDA_SSH", "panda")
PANDA_KEYS = os.environ.get("PANDA_KEYS", "/opt/tnexus/pure_http_keys")
POOL_IDS = ROOT / "reports" / "pool_ids.txt"
OUT = ROOT / "reports" / "key_extract_summary.json"

USABLE_MIN = 8


def ssh_out(remote_cmd: str) -> str:
    r = subprocess.run(
        ["ssh", PANDA, remote_cmd],
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
    )
    return r.stdout


def panda_keys_map() -> dict[int, int]:
    out: dict[int, int] = {}
    for line in ssh_out(f"python3 /root/grok_keys_list.py {PANDA_KEYS}").splitlines():
        parts = line.strip().split("\t")
        if len(parts) >= 2 and parts[0].isdigit():
            out[int(parts[0])] = int(parts[1])
    return out


def panda_enabled() -> dict[str, int]:
    remote = (
        "bash -lc 'set -a; source /opt/tnexus/.env; set +a; "
        "psql \"$GROK_DATABASE_URL\" -At -c "
        "\"SELECT count(*) FILTER (WHERE enabled), count(*) FROM grok_accounts WHERE provider=%s\"'"
    ) % "'grok_web'"
    for line in ssh_out(remote).splitlines():
        parts = line.strip().split("|")
        if len(parts) == 2 and parts[0].isdigit() and parts[1].isdigit():
            return {"enabled": int(parts[0]), "total": int(parts[1])}
    return {"enabled": -1, "total": -1}


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--no-panda", action="store_true", help="skip ssh queries")
    args = ap.parse_args()

    pool = [int(x) for x in POOL_IDS.read_text().split() if x.strip().isdigit()]
    pool_set = set(pool)
    idx_of = {aid: i for i, aid in enumerate(pool)}

    local = scan_local_keys()
    rows = load_progress()

    attempted = {r["account_id"] for r in rows if "account_id" in r}
    usable = {a for a, n in local.items() if n >= USABLE_MIN}
    extracted = set(local) & pool_set
    usable_pool = usable & pool_set

    # 每个账号最后一次尝试
    last: dict[int, dict] = {}
    attempts = collections.Counter()
    for r in rows:
        aid = r.get("account_id")
        if aid is None:
            continue
        last[aid] = r
        attempts[aid] += 1

    # 失败 = 在池内但没有可用 fingerprint
    failed = sorted(pool_set - usable_pool)
    reasons: dict[str, list[int]] = collections.defaultdict(list)
    for aid in failed:
        if aid not in attempted:
            reasons["never_attempted"].append(aid)
        elif aid in local and local[aid] < USABLE_MIN:
            reasons["extracted_but_empty_fingerprint"].append(aid)
        else:
            reasons[err_bucket(last.get(aid, {}).get("error", ""))].append(aid)

    reason_dist = {k: len(v) for k, v in sorted(reasons.items(), key=lambda kv: -len(kv[1]))}

    # 未覆盖的 offset 区间（以 pool 顺序）
    bad_idx = sorted(idx_of[a] for a in failed)
    spans: list[list[int]] = []
    if bad_idx:
        s = prev = bad_idx[0]
        for i in bad_idx[1:]:
            if i != prev + 1:
                spans.append([s, prev])
                s = i
            prev = i
        spans.append([s, prev])

    summary: dict = {
        "generated_at": __import__("time").strftime("%Y-%m-%dT%H:%M:%S"),
        "usable_fingerprint_min_len": USABLE_MIN,
        "pool_total": len(pool),
        "extracted": len(extracted),
        "fingerprint_usable": len(usable_pool),
        "fingerprint_unusable": len(extracted) - len(usable_pool),
        "never_attempted": len(pool_set - attempted),
        "coverage_pct": round(100.0 * len(usable_pool) / max(1, len(pool)), 2),
        "total_attempts": len(rows),
        "failed_count": len(failed),
        "failure_reason_dist": reason_dist,
        "failed_accounts": failed,
        "failed_offset_spans": spans,
    }

    if not args.no_panda:
        pmap = panda_keys_map()
        summary["panda_key_files"] = len(pmap)
        summary["panda_fingerprint_usable"] = sum(1 for v in pmap.values() if v >= USABLE_MIN)
        summary["panda_missing_vs_local"] = sorted(a for a in usable_pool if pmap.get(a, -1) < USABLE_MIN)
        summary["panda_enabled"] = panda_enabled()

    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(json.dumps(summary, ensure_ascii=False, indent=2), encoding="utf-8")

    brief = {k: v for k, v in summary.items() if k not in ("failed_accounts", "failed_offset_spans", "panda_missing_vs_local")}
    print(json.dumps(brief, ensure_ascii=False, indent=2))
    print(f"\nwritten: {OUT}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
