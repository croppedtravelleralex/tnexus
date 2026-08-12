#!/bin/bash
set -euo pipefail
# TNexus-NewAPI (channel 115) gpt-image-2 stats from NewAPI logs + pipeline_events
MODEL="gpt-image-2"
CH="115"

run_window() {
  local label="$1"
  local since="$2"
  echo "=== NewAPI ch${CH} ${label} (model=${MODEL}) ==="
  docker exec new-api-postgres psql -U newapi -d new-api -c "
SELECT channel_id,
       COUNT(*) AS req,
       SUM(CASE WHEN type=2 THEN 1 ELSE 0 END) AS ok,
       SUM(CASE WHEN type!=2 THEN 1 ELSE 0 END) AS fail,
       ROUND(100.0*SUM(CASE WHEN type=2 THEN 1 ELSE 0 END)/NULLIF(COUNT(*),0),2) AS ok_pct,
       ROUND((PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY use_time))::numeric,2) AS p50_s,
       ROUND((PERCENTILE_CONT(0.95) WITHIN GROUP (ORDER BY use_time))::numeric,2) AS p95_s,
       ROUND((PERCENTILE_CONT(0.99) WITHIN GROUP (ORDER BY use_time))::numeric,2) AS p99_s
FROM logs
WHERE model_name='${MODEL}' AND channel_id=${CH} AND created_at >= ${since}
GROUP BY channel_id;
"
  echo "=== Top errors ${label} ==="
  docker exec new-api-postgres psql -U newapi -d new-api -c "
SELECT LEFT(COALESCE(other,''),100) AS err, COUNT(*) AS n
FROM logs
WHERE model_name='${MODEL}' AND channel_id=${CH} AND type!=2 AND created_at >= ${since}
GROUP BY 1 ORDER BY 2 DESC LIMIT 8;
"
}

NOW=$(docker exec new-api-postgres psql -U newapi -d new-api -tAc "SELECT extract(epoch from now())::bigint;")
run_window "last_24h" "$((NOW - 86400))"
run_window "last_7d" "$((NOW - 7*86400))"

# `logs` has no `message` column; the human-readable reason lives in `content`.
echo "=== Error detail last_24h (content) ==="
docker exec new-api-postgres psql -U newapi -d new-api -c "
SELECT LEFT(COALESCE(content,''),160) AS content, COUNT(*) AS n
FROM logs
WHERE model_name='${MODEL}' AND channel_id=${CH} AND type!=2 AND created_at >= $((NOW - 86400))
GROUP BY 1 ORDER BY 2 DESC LIMIT 10;
"

# Channel auto-tests run as root and only ever log successes, inflating the rate.
echo "=== Real-user success rate last_7d (excludes root/auto-test) ==="
docker exec new-api-postgres psql -U newapi -d new-api -c "
SELECT COUNT(*) AS req,
       SUM(CASE WHEN type=2 THEN 1 ELSE 0 END) AS ok,
       ROUND(100.0*SUM(CASE WHEN type=2 THEN 1 ELSE 0 END)/NULLIF(COUNT(*),0),2) AS ok_pct
FROM logs
WHERE model_name='${MODEL}' AND channel_id=${CH}
  AND username <> 'root'
  AND created_at >= $((NOW - 7*86400));
"
echo "=== pipeline_events gateway_image last_7d (TNexus gateway wall ms) ==="
python3 - "$((NOW - 7*86400))" <<'PY'
import json, sys, statistics
from datetime import datetime, timezone
cut = int(sys.argv[1])
path = "/opt/tnexus/data/pool/pipeline_events.ndjson"
rows = []
for line in open(path, encoding="utf-8"):
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
walls = sorted([
    (r.get("timings_ms") or {}).get("gateway_wall_ms")
    for r in ok
    if isinstance((r.get("timings_ms") or {}).get("gateway_wall_ms"), (int, float))
])
n = len(rows)
print(f"events={n} ok={len(ok)} fail={n-len(ok)} ok_pct={100*len(ok)/max(n,1):.1f}%")
if walls:
    def pct(p):
        i = max(0, min(len(walls)-1, int(p * len(walls)) - 1))
        return int(walls[i])
    print(f"gateway_wall_ms p50={pct(0.5)} p95={pct(0.95)} p99={pct(0.99)} avg={int(sum(walls)/len(walls))}")
PY
