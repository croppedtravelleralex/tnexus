#!/usr/bin/env python3
import json
import subprocess

day_start = subprocess.check_output(
    [
        "docker", "exec", "new-api-postgres", "psql", "-U", "newapi", "-d", "new-api", "-tAc",
        "SELECT extract(epoch from timestamptz '2026-08-07 00:00:00+08')::bigint;",
    ],
    text=True,
).strip()

raw = subprocess.check_output(
    [
        "docker", "exec", "new-api-postgres", "psql", "-U", "newapi", "-d", "new-api", "-tAc",
        f"SELECT other FROM logs WHERE channel_id=114 AND model_name='gpt-image-2' "
        f"AND type!=2 AND created_at >= {day_start} ORDER BY created_at DESC LIMIT 3;",
    ],
    text=True,
)

print("=== ch114 今日失败样本 (other JSON) ===")
for i, line in enumerate(raw.splitlines(), 1):
    if not line.strip():
        continue
    try:
        o = json.loads(line)
        print(f"\n--- sample {i} ---")
        for k in ("status_code", "error_code", "error", "message", "frt"):
            if k in o:
                print(f"  {k}: {o[k]}")
        admin = o.get("admin_info") or {}
        for k in ("status_code", "error", "upstream_message"):
            if k in admin:
                print(f"  admin.{k}: {admin[k]}")
    except json.JSONDecodeError:
        print(line[:300])
