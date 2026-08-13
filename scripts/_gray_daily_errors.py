#!/usr/bin/env python3
import re, subprocess
from collections import Counter

def psql(sql):
    return subprocess.check_output(
        ["docker", "exec", "new-api-postgres", "psql", "-U", "newapi", "-d", "new-api", "-tAc", sql],
        text=True,
    )

gray_ts = 1785806300
MODEL = "gpt-image-2"

sql = f"""
SELECT to_char(to_timestamp(l.created_at) AT TIME ZONE 'Asia/Shanghai','MM-DD') as day,
       l.channel_id, COUNT(*),
       SUM(CASE WHEN l.type=2 THEN 1 ELSE 0 END),
       ROUND(100.0*SUM(CASE WHEN l.type=2 THEN 1 ELSE 0 END)/COUNT(*),1)
FROM logs l
WHERE l.model_name='{MODEL}' AND l.channel_id IN (84,114) AND l.created_at>={gray_ts}
GROUP BY day, l.channel_id ORDER BY day, l.channel_id;
"""
print("=== daily ===")
for line in psql(sql).strip().splitlines():
    day, ch, req, ok, pct = line.split("|")
    name = "gptimage" if ch == "84" else "TNexus"
    print(f"  {day} ch{ch}({name}): req={req} ok={ok} ({pct}%)")

raw = psql(
    f"SELECT channel_id||'|'||other FROM logs WHERE channel_id IN (84,114) "
    f"AND model_name='{MODEL}' AND type!=2 AND created_at>={gray_ts}"
)
by = {84: Counter(), 114: Counter()}
for line in raw.splitlines():
    if "|" not in line:
        continue
    ch = int(line.split("|", 1)[0])
    other = line.split("|", 1)[1]
    m = re.search(r'"error_code":"([^"]+)"', other)
    by[ch][m.group(1) if m else "unknown"] += 1
print("\n=== errors since gray ===")
codes = sorted(set(by[84]) | set(by[114]), key=lambda c: -(by[84][c] + by[114][c]))
for c in codes[:12]:
    print(f"  {c:28s} ch84={by[84][c]:4d} ch114={by[114][c]:4d}")

r = psql(
    f"SELECT COUNT(*), SUM(CASE WHEN type=2 THEN 1 ELSE 0 END), "
    f"SUM(CASE WHEN type!=2 THEN 1 ELSE 0 END) FROM logs "
    f"WHERE channel_id=115 AND model_name='{MODEL}' AND created_at>={gray_ts}"
).strip()
print(f"\nch115 dedicated: req|ok|fail = {r}")

PY
