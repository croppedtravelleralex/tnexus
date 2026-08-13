#!/usr/bin/env python3
"""One-shot gray stats from Panda NewAPI + pipeline_events (run on Panda)."""
import json
import statistics
import subprocess
from collections import defaultdict
from datetime import datetime, timezone

def psql(sql: str) -> str:
    return subprocess.check_output(
        [
            "docker", "exec", "new-api-postgres",
            "psql", "-U", "newapi", "-d", "new-api", "-tAc", sql,
        ],
        text=True,
    )


def main() -> None:
    gray_ts = int(psql("SELECT created_time FROM channels WHERE id=114;").strip() or "0")
    now = int(psql("SELECT extract(epoch from now())::bigint;").strip())
    windows = [
        ("since_gray", gray_ts),
        ("last_7d", now - 7 * 86400),
        ("last_24h", now - 86400),
    ]
    model = "gpt-image-2"
    channels = "84,114,115"

    print("gray_channel_created", datetime.fromtimestamp(gray_ts, tz=timezone.utc).isoformat())
    print("current_weights", psql(
        f"SELECT id,name,weight FROM channels WHERE id IN ({channels}) ORDER BY id;"
    ).strip())

    for label, since in windows:
        print(f"\n=== NewAPI {label} ===")
        sql = f"""
        SELECT c.id||'|'||c.name||'|'||c.weight||'|'||
               COUNT(*)||'|'||
               SUM(CASE WHEN l.type=2 THEN 1 ELSE 0 END)||'|'||
               SUM(CASE WHEN l.type!=2 THEN 1 ELSE 0 END)||'|'||
               COALESCE(ROUND(100.0*SUM(CASE WHEN l.type=2 THEN 1 ELSE 0 END)/NULLIF(COUNT(*),0),1)::text,'0')||'|'||
               COALESCE(ROUND(AVG(l.use_time)::numeric,1)::text,'0')||'|'||
               COALESCE(ROUND((PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY l.use_time))::numeric,1)::text,'0')||'|'||
               COALESCE(ROUND((PERCENTILE_CONT(0.9) WITHIN GROUP (ORDER BY l.use_time))::numeric,1)::text,'0')
        FROM logs l JOIN channels c ON c.id=l.channel_id
        WHERE l.channel_id IN ({channels})
          AND l.model_name='{model}'
          AND l.created_at >= {since}
        GROUP BY c.id,c.name,c.weight ORDER BY c.id;
        """
        lines = [ln for ln in psql(sql).strip().splitlines() if ln.strip()]
        total_req = sum(int(ln.split("|")[3]) for ln in lines if "|" in ln)
        for line in lines:
            if not line.strip():
                continue
            parts = line.split("|")
            if len(parts) < 10:
                print(line)
                continue
            cid, name, weight, req, ok, fail, ok_pct, avg_s, p50, p90 = parts[:10]
            share = 100.0 * int(req) / max(1, total_req)
            print(
                f"  ch{cid} {name} weight={weight} req={req} ({share:.1f}% vol) "
                f"ok={ok} fail={fail} ok%={ok_pct} avg={avg_s}s p50={p50}s p90={p90}s"
            )

    print("\n=== NewAPI daily (84 vs 114) since gray ===")
    sql = f"""
    SELECT to_char(to_timestamp(l.created_at) AT TIME ZONE 'Asia/Shanghai','MM-DD')||'|'||
           l.channel_id||'|'||COUNT(*)||'|'||
           SUM(CASE WHEN l.type=2 THEN 1 ELSE 0 END)||'|'||
           ROUND(100.0*SUM(CASE WHEN l.type=2 THEN 1 ELSE 0 END)/COUNT(*),1)
    FROM logs l
    WHERE l.model_name='{model}' AND l.channel_id IN (84,114) AND l.created_at>={gray_ts}
    GROUP BY 1,l.channel_id ORDER BY 1,l.channel_id;
    """
    print(psql(sql))

    print("\n=== TNexus pipeline_events (gateway :8014 direct) last 7d ===")
    path = "/opt/tnexus/data/pool/pipeline_events.ndjson"
    cut = now - 7 * 86400
    rows = []
    with open(path, encoding="utf-8") as f:
        for line in f:
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
            rows.append(o)
    ok = [r for r in rows if r.get("ok")]
    walls = [
        (r.get("timings_ms") or {}).get("gateway_wall_ms")
        for r in ok
        if isinstance((r.get("timings_ms") or {}).get("gateway_wall_ms"), (int, float))
    ]
    walls.sort()
    print(f"  events={len(rows)} ok={len(ok)} fail={len(rows)-len(ok)} ok%={100*len(ok)/max(len(rows),1):.1f}")
    if walls:
        print(
            f"  latency_ms p50={int(statistics.median(walls))} "
            f"p90={int(walls[int(0.9*len(walls))-1])} avg={int(sum(walls)/len(walls))}"
        )


if __name__ == "__main__":
    main()
