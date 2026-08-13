#!/usr/bin/env python3
import re
from pathlib import Path

t = Path(r"D:\SelfMadeTool\TNexus\scripts\.tmp\grok_chunks_live\031yhyjenz-n2.js").read_text(
    encoding="utf-8"
)
seen = set()
for m in re.finditer(r'path:"(/rest/app-chat/[^"]+)"', t):
    seen.add(m.group(1))
for p in sorted(seen):
    print(p)
