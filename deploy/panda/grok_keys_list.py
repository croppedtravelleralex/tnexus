#!/usr/bin/env python3
"""列出 Panda keys 目录中 account_{id}.json 的 fingerprint 长度：每行 `id<TAB>fp_len`。"""
from __future__ import annotations

import json
import os
import re
import sys

KEYS_DIR = sys.argv[1] if len(sys.argv) > 1 else "/opt/tnexus/pure_http_keys"

for name in sorted(os.listdir(KEYS_DIR)):
    m = re.fullmatch(r"account_(\d+)\.json", name)
    if not m:
        continue
    path = os.path.join(KEYS_DIR, name)
    try:
        with open(path, encoding="utf-8") as fh:
            data = json.load(fh)
        fp = str(data.get("fingerprint") or "").strip()
        meta = 1 if data.get("meta_b64") else 0
        print(f"{m.group(1)}\t{len(fp)}\t{meta}")
    except Exception:
        print(f"{m.group(1)}\t-1\t0")
