#!/bin/bash
set -euo pipefail
MODEL="gpt-image-2"
CH="115"
NOW=$(docker exec new-api-postgres psql -U newapi -d new-api -tAc "SELECT extract(epoch from now())::bigint;")

echo "=== request_path breakdown last_24h ==="
docker exec new-api-postgres psql -U newapi -d new-api -c "
SELECT
  CASE
    WHEN other LIKE '%/v1/images/generations%' THEN 'images/generations'
    WHEN other LIKE '%/v1/chat/completions%' THEN 'chat/completions'
    WHEN other LIKE '%/v1/images/edits%' THEN 'images/edits'
    ELSE 'other/unknown'
  END AS path,
  type,
  COUNT(*) AS n
FROM logs
WHERE model_name='${MODEL}' AND channel_id=${CH}
  AND created_at >= $((NOW - 86400))
GROUP BY 1, 2 ORDER BY 3 DESC;"

echo "=== request_path breakdown last_7d ==="
docker exec new-api-postgres psql -U newapi -d new-api -c "
SELECT
  CASE
    WHEN other LIKE '%/v1/images/generations%' THEN 'images/generations'
    WHEN other LIKE '%/v1/chat/completions%' THEN 'chat/completions'
    WHEN other LIKE '%/v1/images/edits%' THEN 'images/edits'
    ELSE 'other/unknown'
  END AS path,
  type,
  COUNT(*) AS n
FROM logs
WHERE model_name='${MODEL}' AND channel_id=${CH}
  AND created_at >= $((NOW - 604800))
GROUP BY 1, 2 ORDER BY 3 DESC;"

echo "=== images/generations only last_24h ==="
docker exec new-api-postgres psql -U newapi -d new-api -c "
SELECT COUNT(*) AS req,
  SUM(CASE WHEN type=2 THEN 1 ELSE 0 END) AS ok,
  SUM(CASE WHEN type!=2 THEN 1 ELSE 0 END) AS fail,
  ROUND(100.0*SUM(CASE WHEN type=2 THEN 1 ELSE 0 END)/NULLIF(COUNT(*),0),2) AS ok_pct,
  ROUND((PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY use_time))::numeric,2) AS p50_s,
  ROUND((PERCENTILE_CONT(0.95) WITHIN GROUP (ORDER BY use_time))::numeric,2) AS p95_s,
  ROUND((PERCENTILE_CONT(0.99) WITHIN GROUP (ORDER BY use_time))::numeric,2) AS p99_s
FROM logs
WHERE model_name='${MODEL}' AND channel_id=${CH}
  AND created_at >= $((NOW - 86400))
  AND other LIKE '%/v1/images/generations%';"

echo "=== images/generations only last_7d ==="
docker exec new-api-postgres psql -U newapi -d new-api -c "
SELECT COUNT(*) AS req,
  SUM(CASE WHEN type=2 THEN 1 ELSE 0 END) AS ok,
  SUM(CASE WHEN type!=2 THEN 1 ELSE 0 END) AS fail,
  ROUND(100.0*SUM(CASE WHEN type=2 THEN 1 ELSE 0 END)/NULLIF(COUNT(*),0),2) AS ok_pct,
  ROUND((PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY use_time))::numeric,2) AS p50_s,
  ROUND((PERCENTILE_CONT(0.95) WITHIN GROUP (ORDER BY use_time))::numeric,2) AS p95_s,
  ROUND((PERCENTILE_CONT(0.99) WITHIN GROUP (ORDER BY use_time))::numeric,2) AS p99_s
FROM logs
WHERE model_name='${MODEL}' AND channel_id=${CH}
  AND created_at >= $((NOW - 604800))
  AND other LIKE '%/v1/images/generations%';"

echo "=== error_code breakdown failures last_24h ==="
docker exec new-api-postgres psql -U newapi -d new-api -c "
SELECT
  substring(other from '\"status_code\":([0-9]+)') AS status_code,
  substring(other from '\"error_code\":\"([^\"]+)\"') AS error_code,
  substring(other from '\"request_path\":\"([^\"]+)\"') AS request_path,
  COUNT(*) AS n
FROM logs
WHERE model_name='${MODEL}' AND channel_id=${CH} AND type!=2
  AND created_at >= $((NOW - 86400))
GROUP BY 1,2,3 ORDER BY 4 DESC;"

echo "=== error_code breakdown failures last_7d ==="
docker exec new-api-postgres psql -U newapi -d new-api -c "
SELECT
  substring(other from '\"status_code\":([0-9]+)') AS status_code,
  substring(other from '\"error_code\":\"([^\"]+)\"') AS error_code,
  substring(other from '\"request_path\":\"([^\"]+)\"') AS request_path,
  COUNT(*) AS n
FROM logs
WHERE model_name='${MODEL}' AND channel_id=${CH} AND type!=2
  AND created_at >= $((NOW - 604800))
GROUP BY 1,2,3 ORDER BY 4 DESC;"

echo "=== sample success rows last_24h ==="
docker exec new-api-postgres psql -U newapi -d new-api -c "
SELECT id, type, use_time, left(COALESCE(other,''),200) AS other_snip
FROM logs
WHERE model_name='${MODEL}' AND channel_id=${CH} AND type=2
  AND created_at >= $((NOW - 86400))
ORDER BY id DESC LIMIT 3;"
