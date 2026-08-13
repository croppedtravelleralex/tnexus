#!/usr/bin/env python3
import re, subprocess
from collections import Counter
gray_ts = subprocess.check_output(
    ["docker", "exec", "new-api-postgres", "psql", "-U", "newapi", "-d", "new-api", "-tAc",
     "SELECT created_time FROM channels WHERE id=114;"], text=True).strip()
raw = subprocess.check_output(
    ["docker", "exec", "new-api-postgres", "psql", "-U", "newapi", "-d", "new-api", "-tAc",
     f"SELECT channel_id||'|'||other FROM logs WHERE channel_id IN (84,114,115) AND model_name='gpt-image-2' AND type!=2 AND created_at>={gray_ts};"],
    text=True)
by_ch = {84: Counter(), 114: Counter(), 115: Counter()}
for line in raw.splitlines():
    if "|" not in line:
        continue
    ch_s, other = line.split("|", 1)
    try:
        ch = int(ch_s)
    except ValueError:
        continue
    m = re.search(r'"error_code":"([^"]+)"', other)
    code = m.group(1) if m else "unknown"
    by_ch.setdefault(ch, Counter())[code] += 1
names = {84: "gptimage:8012", 114: "tnexus-gateway:8014", 115: "tnexus-dedicated"}
for ch in (84, 114, 115):
    print(f"\n=== {names[ch]} errors since gray ===")
    for k, v in by_ch.get(ch, Counter()).most_common(12):
        print(f"  {v:4d} {k}")
