#!/usr/bin/env python3
from __future__ import annotations

import re
from pathlib import Path

ROOT = Path(__file__).resolve().parent / ".tmp" / "grok_chunks_live"


def main() -> None:
    for path in sorted(ROOT.glob("*.js")):
        text = path.read_text(encoding="utf-8", errors="replace")
        if "childNodes" not in text:
            continue
        if "statsig" not in text.lower() and "x-statsig" not in text.lower():
            continue
        ids = re.findall(r",(\d{5,8}),", text)
        uniq = sorted({int(x) for x in ids})
        print(path.name, "ids", uniq[:20], "...", "total", len(uniq))
        for m in re.finditer(r'\.s\(\["default"', text):
            start = max(0, m.start() - 80)
            print("  default_export near:", text[start : m.start() + 30].replace("\n", " ")[:120])
            break


if __name__ == "__main__":
    main()
