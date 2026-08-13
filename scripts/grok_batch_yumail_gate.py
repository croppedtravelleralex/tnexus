#!/usr/bin/env python3
"""yumail 有额度账号批量 extract + gate（支持 Panda udeal 出口）。

用法（本机经 Panda udeal）：
  set GROK_UPSTREAM_PROXY=<udeal_url>
  py -3.12 scripts/grok_batch_yumail_gate.py --domain yumail.co --limit 10

用法（Panda 上，web_auths 在 /tmp/web_auths）：
  GROK_WEB_AUTHS=/tmp/web_auths GROK_KEYS_DIR=/tmp/pure_http_keys \\
    python3 /tmp/grok_batch_yumail_gate.py --skip-quota-scan --emails-file /tmp/yumail_quota.txt
"""
from __future__ import annotations

import argparse
import json
import os
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

from grok_pure_http_client import DEFAULT_OCR_IMAGE, KEYS_DIR, GROK_ROOT, run_gate  # noqa: E402

WEB_AUTHS = Path(os.environ.get("GROK_WEB_AUTHS", str(GROK_ROOT / "web_auths")))
KEYS_OUT = Path(os.environ.get("GROK_KEYS_DIR", str(KEYS_DIR)))
REPORT_DIR = KEYS_OUT / "batch_gate"


def load_quota_scan(path: Path) -> list[str]:
    data = json.loads(path.read_text(encoding="utf-8"))
    out: list[str] = []
    for row in data.get("results") or []:
        if not row.get("ok"):
            continue
        rem = row.get("remainingQueries")
        if rem is None or int(rem) < 1:
            continue
        out.append(str(row["email"]))
    return out


def list_from_scan_or_domain(domain: str, limit: int, min_remaining: int, scan_json: Path | None) -> list[str]:
    if scan_json and scan_json.exists():
        emails = load_quota_scan(scan_json)
        return emails[:limit]
    # fallback: alphabetical yumail files (caller should run quota scan first)
    files = sorted(WEB_AUTHS.glob("*.json"))
    out: list[str] = []
    for f in files:
        email = f.stem
        if domain and not email.endswith(domain):
            continue
        out.append(email)
        if len(out) >= limit:
            break
    return out


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--domain", default="yumail.co")
    ap.add_argument("--limit", type=int, default=10)
    ap.add_argument("--min-remaining", type=int, default=1)
    ap.add_argument("--quota-scan-json", type=Path, help="quota_scan_*.json from grok_account_quota_scan.py")
    ap.add_argument("--emails-file", type=Path, help="one email per line")
    ap.add_argument("--skip-extract", action="store_true", help="keys 已存在则跳过 extract")
    ap.add_argument("--skip-quota-scan", action="store_true")
    ap.add_argument("--signer", default="auto", choices=("auto", "python", "node"))
    ap.add_argument("--image", type=Path, default=None)
    ap.add_argument("--sleep", type=float, default=1.0)
    args = ap.parse_args()

    if args.emails_file and args.emails_file.exists():
        emails = [ln.strip() for ln in args.emails_file.read_text(encoding="utf-8").splitlines() if ln.strip()]
    else:
        emails = list_from_scan_or_domain(args.domain, args.limit, args.min_remaining, args.quota_scan_json)

    if not emails:
        print("no emails", file=sys.stderr)
        return 1

    REPORT_DIR.mkdir(parents=True, exist_ok=True)
    summary: list[dict] = []
    t0 = time.time()

    image_path = args.image
    if image_path is None and Path("/tmp/grok_ocr_probe.png").exists():
        image_path = Path("/tmp/grok_ocr_probe.png")
    elif image_path is None:
        image_path = DEFAULT_OCR_IMAGE

    for i, email in enumerate(emails):
        keys_path = KEYS_OUT / f"{email.replace('@', '_at_')}.json"
        row: dict = {"email": email, "i": i}
        try:
            need_extract = not args.skip_extract or not keys_path.exists()
            report = run_gate(
                email,
                extract=need_extract,
                headed=False,
                signer=args.signer,
                keys_path=keys_path,
                image_path=image_path,
            )
            row.update(
                {
                    "ok": report.get("ok"),
                    "ocr_ok": report.get("ocr_ok"),
                    "upload_ok": report.get("upload_ok"),
                    "followup_ok": report.get("followup_ok"),
                    "extracted": need_extract,
                }
            )
        except Exception as exc:
            row.update({"ok": False, "error": f"{type(exc).__name__}:{exc}"})
        summary.append(row)
        print(json.dumps(row, ensure_ascii=False), flush=True)
        time.sleep(args.sleep)

    out = {
        "domain": args.domain,
        "upstream_proxy": os.environ.get("GROK_UPSTREAM_PROXY", "")[:24] + "..." if os.environ.get("GROK_UPSTREAM_PROXY") else "",
        "local_proxy": os.environ.get("GROK_LOCAL_PROXY", ""),
        "n": len(summary),
        "ok": sum(1 for r in summary if r.get("ok")),
        "ocr_ok": sum(1 for r in summary if r.get("ocr_ok")),
        "elapsed_s": round(time.time() - t0, 1),
        "results": summary,
    }
    out_path = REPORT_DIR / f"batch_gate_{time.strftime('%Y%m%d-%H%M%S')}.json"
    out_path.write_text(json.dumps(out, ensure_ascii=False, indent=2), encoding="utf-8")
    print(json.dumps({"ok": out["ok"], "n": out["n"], "json": str(out_path)}, ensure_ascii=False))
    return 0 if out["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
