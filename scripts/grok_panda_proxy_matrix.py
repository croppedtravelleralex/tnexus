#!/usr/bin/env python3
"""Panda 四路代理 gate 矩阵（Python 探针；Rust 见 grok_panda_proxy_matrix.sh）。

用法：
  GROK_LOCAL_PROXY=http://127.0.0.1:7897 \\
  python3 scripts/grok_panda_proxy_matrix.py \\
    --keys /opt/grok2api/data/pure_http_keys/nancybaker2jyy_at_yumail.co.json
"""
from __future__ import annotations

import argparse
import json
import os
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

from grok_pure_http_client import DEFAULT_OCR_IMAGE, run_gate  # noqa: E402


def pick_first_proxy(path: Path) -> str | None:
    if not path.exists():
        return None
    for line in path.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        if line.startswith("http"):
            return line
        if "@" in line:
            return f"http://{line}"
        parts = line.split(":")
        if len(parts) == 4:
            host, port, user, pwd = parts
            return f"http://{user}:{pwd}@{host}:{port}"
        if len(parts) == 2:
            return f"http://{parts[0]}:{parts[1]}"
    return None


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--email", default="nancybaker2jyy@yumail.co")
    ap.add_argument("--keys")
    ap.add_argument("--image", type=Path, default=DEFAULT_OCR_IMAGE)
    ap.add_argument("--out-dir", type=Path, default=Path("/tmp/grok_proxy_matrix"))
    ap.add_argument("--signer", default="auto", choices=("auto", "python", "node"))
    args = ap.parse_args()

    out_dir = args.out_dir
    out_dir.mkdir(parents=True, exist_ok=True)

    cases: list[tuple[str, str | None]] = [("direct", None)]
    udeal = os.environ.get("GROK_EGRESS_PROXY", "").strip()
    if udeal:
        cases.append(("udeal", udeal))
    dc = pick_first_proxy(Path("/opt/tnexus/webshare-dc-proxies.txt"))
    if dc:
        cases.append(("webshare_dc", dc))
    res = pick_first_proxy(Path("/opt/tnexus/webshare-proxies.txt"))
    if res:
        cases.append(("webshare_residential", res))

    summary = []
    for label, upstream in cases:
        prev = os.environ.get("GROK_UPSTREAM_PROXY")
        if upstream:
            os.environ["GROK_UPSTREAM_PROXY"] = upstream
        else:
            os.environ.pop("GROK_UPSTREAM_PROXY", None)
        try:
            report = run_gate(
                args.email,
                extract=False,
                headed=False,
                signer=args.signer,
                image_path=args.image if args.image.exists() else None,
                keys_path=Path(args.keys) if args.keys else None,
            )
            report["proxy_label"] = label
            report["upstream_proxy"] = upstream or ""
            out = out_dir / f"gate_{label}.json"
            out.write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
            summary.append(
                {
                    "label": label,
                    "ok": report.get("ok"),
                    "followup_ok": report.get("followup_ok"),
                    "ocr_ok": report.get("ocr_ok"),
                    "upload_ok": report.get("upload_ok"),
                }
            )
            print(json.dumps({"case": label, **summary[-1]}, ensure_ascii=False))
        except Exception as exc:
            summary.append({"label": label, "ok": False, "error": str(exc)})
            print(json.dumps({"case": label, "ok": False, "error": str(exc)}, ensure_ascii=False))
        finally:
            if prev is None:
                os.environ.pop("GROK_UPSTREAM_PROXY", None)
            else:
                os.environ["GROK_UPSTREAM_PROXY"] = prev

    (out_dir / "summary.json").write_text(
        json.dumps(summary, ensure_ascii=False, indent=2), encoding="utf-8"
    )
    return 0 if any(s.get("ok") for s in summary) else 1


if __name__ == "__main__":
    raise SystemExit(main())
