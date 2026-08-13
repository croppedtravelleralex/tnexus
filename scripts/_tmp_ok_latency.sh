#!/bin/bash
docker exec new-api-postgres psql -U newapi -d new-api -c "
SELECT 'ok_images_gen_24h' AS label, COUNT(*) AS ok_n,
  ROUND((PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY use_time))::numeric,2) AS p50_s,
  ROUND((PERCENTILE_CONT(0.95) WITHIN GROUP (ORDER BY use_time))::numeric,2) AS p95_s,
  ROUND((PERCENTILE_CONT(0.99) WITHIN GROUP (ORDER BY use_time))::numeric,2) AS p99_s
FROM logs
WHERE model_name='gpt-image-2' AND channel_id=115 AND type=2
  AND created_at >= extract(epoch from now())::bigint - 86400
  AND other LIKE '%/v1/images/generations%'
UNION ALL
SELECT 'ok_images_gen_7d', COUNT(*),
  ROUND((PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY use_time))::numeric,2),
  ROUND((PERCENTILE_CONT(0.95) WITHIN GROUP (ORDER BY use_time))::numeric,2),
  ROUND((PERCENTILE_CONT(0.99) WITHIN GROUP (ORDER BY use_time))::numeric,2)
FROM logs
WHERE model_name='gpt-image-2' AND channel_id=115 AND type=2
  AND created_at >= extract(epoch from now())::bigint - 604800
  AND other LIKE '%/v1/images/generations%';"
