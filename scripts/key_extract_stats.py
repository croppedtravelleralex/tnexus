#!/usr/bin/env python3
"""本地统计：key 提取进度、fingerprint 质量、失败原因分布。"""
from __future__ import annotations

import argparse
import collections
import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
PROGRESS = ROOT / "reports" / "key_extract_progress.jsonl"
KEYS_DIR = ROOT / "reports" / "pure_http_keys"


def load_progress() -> list[dict]:
    rows = []
    if not PROGRESS.exists():
        return rows
    for line in PROGRESS.read_text(encoding="utf-8", errors="replace").splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            rows.append(json.loads(line))
        except Exception:
            continue
    return rows


def scan_local_keys() -> dict[int, int]:
    """account_id -> fingerprint length (本地 key 文件为准)。"""
    out: dict[int, int] = {}
    if not KEYS_DIR.exists():
        return out
    for p in KEYS_DIR.glob("account_*.json"):
        m = re.fullmatch(r"account_(\d+)\.json", p.name)
        if not m:
            continue
        aid = int(m.group(1))
        try:
            data = json.loads(p.read_text(encoding="utf-8"))
        except Exception:
            out[aid] = -1
            continue
        fp = (data.get("fingerprint") or "").strip()
        meta_ok = bool(data.get("meta_b64"))
        out[aid] = len(fp) if meta_ok else -len(fp) if fp else 0
    return out


def err_bucket(err: str) -> str:
    e = err or ""
    for pat, name in (
        (r"ERR_CONNECTION_CLOSED", "ERR_CONNECTION_CLOSED"),
        (r"ERR_CONNECTION_RESET", "ERR_CONNECTION_RESET"),
        (r"ERR_TIMED_OUT|ERR_CONNECTION_TIMED_OUT", "ERR_TIMED_OUT"),
        (r"ERR_PROXY_CONNECTION_FAILED", "ERR_PROXY_CONNECTION_FAILED"),
        (r"ERR_TUNNEL_CONNECTION_FAILED", "ERR_TUNNEL_CONNECTION_FAILED"),
        (r"ERR_EMPTY_RESPONSE", "ERR_EMPTY_RESPONSE"),
        (r"ERR_NETWORK_CHANGED", "ERR_NETWORK_CHANGED"),
        (r"Timeout .*exceeded|TimeoutError", "PlaywrightTimeout"),
        (r"empty sso", "empty_sso"),
        (r"no rows|not found", "no_credential_row"),
        (r"CalledProcessError", "ssh_panda_error"),
        (r"Target page, context or browser has been closed", "browser_closed"),
        (r"BrowserType.launch|Executable doesn't exist", "browser_launch"),
    ):
        if re.search(pat, e, re.I):
            return name
    return (e.split(":", 1)[0] or "unknown")[:60]


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--pool-ids", default="", help="file with all pool account ids (one per line)")
    ap.add_argument("--json-out", default="")
    ap.add_argument("--list-bad", action="store_true", help="print account ids with unusable fingerprint")
    args = ap.parse_args()

    rows = load_progress()
    local = scan_local_keys()

    attempted = {r["account_id"] for r in rows if "account_id" in r}
    ok_ids = {r["account_id"] for r in rows if r.get("ok")}
    good = {a for a, n in local.items() if n >= 8}
    bad_local = {a for a, n in local.items() if n < 8}

    # 最新一次尝试的失败原因
    last: dict[int, dict] = {}
    for r in rows:
        aid = r.get("account_id")
        if aid is not None:
            last[aid] = r
    fail_last = {a: r for a, r in last.items() if not r.get("ok")}
    err_dist = collections.Counter(err_bucket(r.get("error", "")) for r in fail_last.values())

    pool: list[int] = []
    if args.pool_ids and Path(args.pool_ids).exists():
        pool = [int(x) for x in Path(args.pool_ids).read_text().split() if x.strip().isdigit()]

    result = {
        "progress_lines": len(rows),
        "attempted_unique": len(attempted),
        "ok_unique": len(ok_ids),
        "local_key_files": len(local),
        "local_fp_usable": len(good),
        "local_fp_unusable": len(bad_local),
        "failed_last_attempt": len(fail_last),
        "error_dist": dict(err_dist.most_common()),
    }
    if pool:
        pool_set = set(pool)
        result["pool_total"] = len(pool)
        result["pool_usable"] = len(pool_set & good)
        result["pool_untouched"] = sorted(pool_set - attempted)
        result["pool_untouched_n"] = len(pool_set - attempted)
        missing_idx = [i for i, a in enumerate(pool) if a not in good]
        result["pool_not_usable_n"] = len(missing_idx)
        if missing_idx:
            spans, s = [], missing_idx[0]
            prev = missing_idx[0]
            for i in missing_idx[1:]:
                if i != prev + 1:
                    spans.append([s, prev])
                    s = i
                prev = i
            spans.append([s, prev])
            result["pool_not_usable_offset_spans"] = spans[:60]

    print(json.dumps(result, ensure_ascii=False, indent=2))

    if args.list_bad:
        bad_pool = sorted(bad_local | (set(pool) - good - set()) if pool else bad_local)
        print("BAD_IDS=" + ",".join(str(a) for a in bad_pool))

    if args.json_out:
        Path(args.json_out).write_text(json.dumps(result, ensure_ascii=False, indent=2), encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
