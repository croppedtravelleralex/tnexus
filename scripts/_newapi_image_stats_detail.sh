#!/bin/bash
set -euo pipefail
MODEL="gpt-image-2"
CH="115"
NOW=$(docker exec new-api-postgres psql -U newapi -d new-api -tAc "SELECT extract(epoch from now())::bigint;")

echo "=== logs schema ==="
docker exec new-api-postgres psql -U newapi -d new-api -c "\d logs" | head -50

echo "=== type breakdown last_24h ==="
docker exec new-api-postgres psql -U newapi -d new-api -c "
SELECT type, COUNT(*) AS n
FROM logs
WHERE model_name='${MODEL}' AND channel_id=${CH}
  AND created_at >= $((NOW - 86400))
GROUP BY type ORDER BY n DESC;"

echo "=== type breakdown last_7d ==="
docker exec new-api-postgres psql -U newapi -d new-api -c "
SELECT type, COUNT(*) AS n
FROM logs
WHERE model_name='${MODEL}' AND channel_id=${CH}
  AND created_at >= $((NOW - 604800))
GROUP BY type ORDER BY n DESC;"

echo "=== success-only latency last_24h (use_time seconds) ==="
docker exec new-api-postgres psql -U newapi -d new-api -c "
SELECT COUNT(*) AS ok_n,
  ROUND((PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY use_time))::numeric,2) AS p50_s,
  ROUND((PERCENTILE_CONT(0.95) WITHIN GROUP (ORDER BY use_time))::numeric,2) AS p95_s,
  ROUND((PERCENTILE_CONT(0.99) WITHIN GROUP (ORDER BY use_time))::numeric,2) AS p99_s
FROM logs
WHERE model_name='${MODEL}' AND channel_id=${CH} AND type=2
  AND created_at >= $((NOW - 86400));"

echo "=== success-only latency last_7d (use_time seconds) ==="
docker exec new-api-postgres psql -U newapi -d new-api -c "
SELECT COUNT(*) AS ok_n,
  ROUND((PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY use_time))::numeric,2) AS p50_s,
  ROUND((PERCENTILE_CONT(0.95) WITHIN GROUP (ORDER BY use_time))::numeric,2) AS p95_s,
  ROUND((PERCENTILE_CONT(0.99) WITHIN GROUP (ORDER BY use_time))::numeric,2) AS p99_s
FROM logs
WHERE model_name='${MODEL}' AND channel_id=${CH} AND type=2
  AND created_at >= $((NOW - 604800));"

echo "=== sample failures last_24h ==="
docker exec new-api-postgres psql -U newapi -d new-api -c "
SELECT id, type, use_time, left(COALESCE(other,''),250) AS other_snip
FROM logs
WHERE model_name='${MODEL}' AND channel_id=${CH} AND type!=2
  AND created_at >= $((NOW - 86400))
ORDER BY id DESC LIMIT 5;"

echo "=== channel info ==="
docker exec new-api-postgres psql -U newapi -d new-api -c "
SELECT id, name, status, type FROM channels WHERE id=${CH};"

echo "=== user_image_records last_7d (tnexus postgres) ==="
docker exec panda-postgres-1 psql -U tnexus -d tnexus -c "
SELECT COUNT(*) AS total,
  COUNT(*) FILTER (WHERE created_at >= now() - interval '24 hours') AS last_24h,
  COUNT(*) FILTER (WHERE created_at >= now() - interval '7 days') AS last_7d
FROM user_image_records
WHERE source='gateway_openapi';"
