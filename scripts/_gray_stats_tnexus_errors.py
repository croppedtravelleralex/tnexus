#!/usr/bin/env python3
"""Extended gray stats + TNexus error breakdown (run on Panda)."""
from __future__ import annotations

import json
import re
import subprocess
from collections import Counter, defaultdict
from datetime import datetime, timezone

MODEL = "gpt-image-2"
CHANNELS = "84,114,115"


def psql(sql: str) -> str:
    return subprocess.check_output(
        [
            "docker",
            "exec",
            "new-api-postgres",
            "psql",
            "-U",
            "newapi",
            "-d",
            "new-api",
            "-tAc",
            sql,
        ],
        text=True,
    )


def main() -> None:
    gray_ts = int(psql("SELECT created_time FROM channels WHERE id=114;").strip() or "0")
    now = int(psql("SELECT extract(epoch from now())::bigint;").strip())
    print("gray_start_utc", datetime.fromtimestamp(gray_ts, tz=timezone.utc).isoformat())
    print("weights", psql(f"SELECT id||':'||weight FROM channels WHERE id IN ({CHANNELS}) ORDER BY id;").strip())

    for label, since in [("since_gray", gray_ts), ("last_24h", now - 86400), ("last_7d", now - 7 * 86400)]:
        print(f"\n=== NewAPI {label} (model={MODEL}) ===")
        sql = f"""
        SELECT c.id||'|'||c.name||'|'||c.weight||'|'||
               COUNT(*)||'|'||
               SUM(CASE WHEN l.type=2 THEN 1 ELSE 0 END)||'|'||
               SUM(CASE WHEN l.type!=2 THEN 1 ELSE 0 END)||'|'||
               COALESCE(ROUND(100.0*SUM(CASE WHEN l.type=2 THEN 1 ELSE 0 END)/NULLIF(COUNT(*),0),1)::text,'0')||'|'||
               COALESCE(ROUND((PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY l.use_time))::numeric,1)::text,'0')||'|'||
               COALESCE(ROUND((PERCENTILE_CONT(0.9) WITHIN GROUP (ORDER BY l.use_time))::numeric,1)::text,'0')
        FROM logs l JOIN channels c ON c.id=l.channel_id
        WHERE l.channel_id IN ({CHANNELS}) AND l.model_name='{MODEL}' AND l.created_at >= {since}
        GROUP BY c.id,c.name,c.weight ORDER BY c.id;
        """
        for line in psql(sql).strip().splitlines():
            if not line.strip():
                continue
            p = line.split("|")
            if len(p) >= 9:
                print(
                    f"  ch{p[0]} {p[1]} w={p[2]} req={p[3]} ok={p[4]} fail={p[5]} ok%={p[6]} p50={p[7]}s p90={p[8]}s"
                )

    print("\n=== TNexus ch114/115 error top (since gray) ===")
    sql = f"""
    SELECT LEFT(COALESCE(other,''), 180), COUNT(*)
    FROM logs
    WHERE channel_id IN (114,115) AND model_name='{MODEL}' AND type!=2 AND created_at>={gray_ts}
    GROUP BY 1 ORDER BY 2 DESC LIMIT 25;
    """
    print(psql(sql))

    print("\n=== gptimage ch84 error top (since gray) ===")
    sql = f"""
    SELECT LEFT(COALESCE(other,''), 180), COUNT(*)
    FROM logs
    WHERE channel_id=84 AND model_name='{MODEL}' AND type!=2 AND created_at>={gray_ts}
    GROUP BY 1 ORDER BY 2 DESC LIMIT 15;
    """
    print(psql(sql))

    print("\n=== pipeline_events gateway_image (7d) ===")
    path = "/opt/tnexus/data/pool/pipeline_events.ndjson"
    cut = now - 7 * 86400
    ok = fail = 0
    err_c: Counter[str] = Counter()
    for line in open(path, encoding="utf-8"):
        line = line.strip()
        if not line:
            continue
        try:
            o = json.loads(line)
        except json.JSONDecodeError:
            continue
        if o.get("kind") != "gateway_image":
            continue
        ts = o.get("ts", "")
        try:
            t = datetime.fromisoformat(ts.replace("Z", "+00:00")).timestamp()
        except ValueError:
            continue
        if t < cut:
            continue
        if o.get("ok"):
            ok += 1
        else:
            fail += 1
            err = str(o.get("error") or o.get("message") or o.get("upstream_error") or "unknown")[:120]
            err_c[err] += 1
    total = ok + fail
    print(f"events={total} ok={ok} fail={fail} ok%={100*ok/max(total,1):.1f}")
    for e, n in err_c.most_common(15):
        print(f"  {n}x {e}")


if __name__ == "__main__":
    main()
