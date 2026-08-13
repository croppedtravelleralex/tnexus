#!/bin/bash
DEPLOY_TS=$(date -d "$(docker inspect -f '{{.State.StartedAt}}' panda-gateway-1)" +%s)
docker exec new-api-postgres psql -U newapi -d new-api -c "
SELECT to_timestamp(created_at) AT TIME ZONE 'UTC' AS ts_utc, type, use_time,
       LEFT(other, 800) AS detail
FROM logs
WHERE channel_id=114 AND model_name='gpt-image-2' AND type!=2 AND created_at>=$DEPLOY_TS
ORDER BY created_at DESC LIMIT 5;
"
docker logs panda-gateway-1 --since "$(docker inspect -f '{{.State.StartedAt}}' panda-gateway-1)" 2>&1 | grep -E 'image call failed|image batch bridge failed' | tail -5
