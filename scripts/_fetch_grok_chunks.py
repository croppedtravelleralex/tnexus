#!/usr/bin/env python3
"""Quick fetch grok.com chunks + scan for turbopack signer module IDs."""
from __future__ import annotations

import re
import sys
from pathlib import Path

from curl_cffi import requests as cr

PROXY = "http://127.0.0.1:7897"
UA = (
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 "
    "(KHTML, like Gecko) Chrome/146.0.0.0 Safari/537.36"
)
OUT = Path(__file__).resolve().parent / ".tmp" / "grok_chunks_live"
OUT.mkdir(parents=True, exist_ok=True)


def main() -> int:
    r = cr.get(
        "https://grok.com/",
        headers={"User-Agent": UA},
        proxies={"http": PROXY, "https": PROXY},
        impersonate="chrome131",
        timeout=60,
    )
    print("status", r.status_code, "len", len(r.text))
    html = r.text
    for pat in (
        r'name=["\']grok-site[^"\']*["\'][^>]+content=["\']([^"\']+)',
        r'name=["\']twitter:site[^"\']*["\'][^>]+content=["\']([^"\']+)',
    ):
        m = re.search(pat, html, re.I)
        if m:
            print("meta", m.group(1)[:60])
            break

    chunk_paths = sorted(set(re.findall(r"/_next/static/chunks/[^\"'\s>]+\.js", html)))
    print("chunk_urls", len(chunk_paths))
    hits: dict[int, list[str]] = {}
    for rel in chunk_paths:
        url = "https://grok.com" + rel
        name = rel.rsplit("/", 1)[-1]
        dest = OUT / name
        if not dest.is_file():
            try:
                c = cr.get(
                    url,
                    headers={"User-Agent": UA, "Referer": "https://grok.com/"},
                    proxies={"http": PROXY, "https": PROXY},
                    impersonate="chrome131",
                    timeout=90,
                )
                dest.write_bytes(c.content)
            except Exception as exc:
                print("fail", name, exc)
                continue
        text = dest.read_text(encoding="utf-8", errors="replace")
        for mid in re.findall(r",(\d{5,8}),", text):
            n = int(mid)
            if n in (2347272, 4629918, 1645000, 1645):
                hits.setdefault(n, []).append(name)
        if "childNodes" in text and "statsig" in text.lower():
            hits.setdefault(-1, []).append(f"childNodes+statsig:{name}")
        if re.search(r'\.s\(\["default"', text):
            hits.setdefault(-2, []).append(f"default_export:{name}")

    print("hits", {k: v[:5] for k, v in hits.items()})
    return 0


if __name__ == "__main__":
    sys.exit(main())
