#!/usr/bin/env python3
"""Rebuild grok_sign_standalone.js from live 1645e3 turbopack chunk."""
from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

from curl_cffi import requests as cr

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "crates" / "grok-signer" / "assets" / "grok_sign_standalone.js"
CHUNK_URL = "https://grok.com/_next/static/chunks/1nf91r5--cp6_.js"
PROXY = "http://127.0.0.1:7897"
UA = (
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 "
    "(KHTML, like Gecko) Chrome/146.0.0.0 Safari/537.36"
)


def fetch_chunk(dest: Path) -> str:
    if dest.is_file():
        return dest.read_text(encoding="utf-8")
    r = cr.get(
        CHUNK_URL,
        headers={"User-Agent": UA, "Referer": "https://grok.com/"},
        proxies={"http": PROXY, "https": PROXY},
        impersonate="chrome131",
        timeout=90,
    )
    r.raise_for_status()
    dest.parent.mkdir(parents=True, exist_ok=True)
    dest.write_text(r.text, encoding="utf-8")
    return r.text


def rebuild(bundle: Path, chunk_text: str) -> None:
    chunk_path = bundle.parent / "grok_sign_module_1645e3.js"
    chunk_path.write_text(chunk_text.strip() + "\n", encoding="utf-8")
    if not (bundle.parent / "grok_sign_standalone.js").is_file():
        raise SystemExit(f"missing runner {bundle}")
    # Ensure runner uses external module file (patch if legacy inline src)
    runner = bundle.read_text(encoding="utf-8")
    if "grok_sign_module_1645e3.js" not in runner:
        import subprocess

        subprocess.run([sys.executable, str(Path(__file__).parent / "_patch_signer_runner.py")], check=True)
    print(f"wrote module {chunk_path} ({chunk_path.stat().st_size} bytes)")


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--bundle", type=Path, default=OUT)
    ap.add_argument("--chunk-file", type=Path, default=Path(__file__).parent / ".tmp" / "grok_chunks_live" / "1nf91r5--cp6_.js")
    ap.add_argument("--fetch", action="store_true")
    args = ap.parse_args()
    if args.fetch or not args.chunk_file.is_file():
        fetch_chunk(args.chunk_file)
    chunk = args.chunk_file.read_text(encoding="utf-8")
    rebuild(args.bundle, chunk)
    return 0


if __name__ == "__main__":
    sys.exit(main())
