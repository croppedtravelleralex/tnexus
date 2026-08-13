#!/usr/bin/env python3
import subprocess

sql = """
SELECT to_char(to_timestamp(created_at) AT TIME ZONE 'Asia/Shanghai','MM-DD HH24:MI') as t,
       COUNT(*) FILTER (WHERE type=2) as ok,
       COUNT(*) FILTER (WHERE type!=2) as fail
FROM logs
WHERE channel_id=114 AND model_name='gpt-image-2'
  AND created_at >= extract(epoch from timestamptz '2026-08-07 00:00:00+08')::bigint
GROUP BY to_char(to_timestamp(created_at) AT TIME ZONE 'Asia/Shanghai','MM-DD HH24:MI')
ORDER BY t;
"""
out = subprocess.check_output(
    ["docker", "exec", "new-api-postgres", "psql", "-U", "newapi", "-d", "new-api", "-c", sql],
    text=True,
)
print("=== ch114 今日按小时 ===")
print(out)
