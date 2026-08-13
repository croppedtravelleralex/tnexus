#!/bin/bash
set -euo pipefail
DEPLOY_ISO=$(docker inspect -f '{{.State.StartedAt}}' panda-gateway-1)
DEPLOY_TS=$(date -d "$DEPLOY_ISO" +%s)
echo "=== DEPLOY gateway StartedAt: $DEPLOY_ISO epoch=$DEPLOY_TS ==="

echo "=== NewAPI gpt-image-2 ch114/115 since deploy ==="
docker exec new-api-postgres psql -U newapi -d new-api -c "
SELECT c.id, c.name,
       COUNT(*) AS req,
       SUM(CASE WHEN l.type=2 THEN 1 ELSE 0 END) AS ok,
       SUM(CASE WHEN l.type!=2 THEN 1 ELSE 0 END) AS fail,
       ROUND(100.0*SUM(CASE WHEN l.type=2 THEN 1 ELSE 0 END)/NULLIF(COUNT(*),0),1) AS ok_pct,
       ROUND((PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY l.use_time))::numeric,1) AS p50_s
FROM logs l JOIN channels c ON c.id=l.channel_id
WHERE l.model_name='gpt-image-2'
  AND l.channel_id IN (114,115)
  AND l.created_at >= $DEPLOY_TS
GROUP BY c.id, c.name ORDER BY c.id;
"

echo "=== Error top since deploy ==="
docker exec new-api-postgres psql -U newapi -d new-api -c "
SELECT LEFT(COALESCE(other,''), 120) AS err, COUNT(*) AS n
FROM logs
WHERE channel_id IN (114,115) AND model_name='gpt-image-2'
  AND type!=2 AND created_at>=$DEPLOY_TS
GROUP BY 1 ORDER BY 2 DESC LIMIT 20;
"

echo "=== Gateway docker log image failures ==="
docker logs panda-gateway-1 --since "$DEPLOY_ISO" 2>&1 | grep -cE 'image call failed|image batch bridge failed' || true

echo "=== pipeline_events gateway_image since deploy ==="
python3 - "$DEPLOY_ISO" <<'PY'
import json, sys
from datetime import datetime
deploy = datetime.fromisoformat(sys.argv[1].replace("Z", "+00:00"))
path = "/opt/tnexus/data/pool/pipeline_events.ndjson"
rows = []
for line in open(path, encoding="utf-8"):
    o = json.loads(line)
    if o.get("kind") != "gateway_image":
        continue
    ts = datetime.fromisoformat(o["ts"].replace("Z", "+00:00"))
    if ts >= deploy:
        rows.append(o)
ok = [r for r in rows if r.get("ok")]
walls = [(r.get("timings_ms") or {}).get("gateway_wall_ms") for r in ok]
walls = [w for w in walls if isinstance(w, (int, float))]
walls.sort()
print(f"events={len(rows)} ok={len(ok)} (gateway failures not in ndjson)")
if walls:
    print(f"p50_ms={int(walls[len(walls)//2])} p90_ms={int(walls[int(0.9*len(walls))-1])}")
PY

echo "=== user_image_records since deploy ==="
docker exec panda-postgres-1 psql -U tnexus -d tnexus -c "
SELECT COUNT(*) AS api_images, MIN(created_at) AS first, MAX(created_at) AS last
FROM user_image_records
WHERE source='gateway_openapi' AND created_at >= '$DEPLOY_ISO'::timestamptz;
"

echo "=== grok imagine audits since grok2api deploy (if any) ==="
GROK_CID=$(docker ps -qf name=grok2api-rs 2>/dev/null || true)
if [ -n "$GROK_CID" ]; then
  GROK_ISO=$(docker inspect -f '{{.State.StartedAt}}' "$GROK_CID")
  docker exec panda-postgres-1 psql -U tnexus -d tnexus -c "
SELECT status_code, error_code, COUNT(*)
FROM grok_request_audits
WHERE operation='image' AND created_at >= '$GROK_ISO'::timestamptz
GROUP BY 1,2 ORDER BY 3 DESC LIMIT 15;
"
else
  echo "grok2api-rs not running"
fi
