#!/usr/bin/env python3
import re
from pathlib import Path

t = Path(r"D:\SelfMadeTool\TNexus\crates\grok-signer\assets\grok_sign_module_1645e3.js").read_text(
    encoding="utf-8"
)
for m in re.finditer(r'"([^"\\]{3,120})"', t):
    s = m.group(1)
    if any(x in s for x in ("gr", "meta", "query", "child", "r-", "site", "verif", "rest", "POST", "GET")):
        print(s)
i = t.find(",1645e3,")
print("\n--- factory head ---\n")
print(t[i : i + 3200])
