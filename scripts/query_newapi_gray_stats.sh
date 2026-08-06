#!/usr/bin/env bash
# NewAPI gray usage: channel 84 (:8012) vs 114 (:8014 tnexus-gateway)
set -euo pipefail
PG=new-api-postgres
docker exec "$PG" psql -U newapi -d new-api -v ON_ERROR_STOP=1 <<'SQL'
-- table discovery
SELECT tablename FROM pg_tables WHERE schemaname='public' AND tablename IN ('logs','consume_logs','quota_data') ORDER BY 1;

-- channel weights now
SELECT id, name, base_url, weight, status FROM channels WHERE id IN (84,114) ORDER BY id;

-- logs schema if exists
SELECT column_name FROM information_schema.columns WHERE table_name='logs' ORDER BY ordinal_position;

-- usage since gray (channel 114 created ~2026-08-04)
SELECT channel_id,
       COUNT(*) AS requests,
       SUM(CASE WHEN type=2 THEN 1 ELSE 0 END) AS success_cnt,
       SUM(CASE WHEN type!=2 THEN 1 ELSE 0 END) AS fail_cnt,
       ROUND(AVG(use_time)::numeric, 2) AS avg_use_time_ms,
       MAX(created_at) AS last_at
FROM logs
WHERE channel_id IN (84, 114)
  AND created_at >= '2026-08-03 00:00:00'
GROUP BY channel_id
ORDER BY channel_id;

-- daily breakdown
SELECT channel_id,
       DATE(created_at) AS day,
       COUNT(*) AS cnt,
       SUM(CASE WHEN type=2 THEN 1 ELSE 0 END) AS ok
FROM logs
WHERE channel_id IN (84, 114)
  AND created_at >= '2026-08-03 00:00:00'
GROUP BY channel_id, DATE(created_at)
ORDER BY day, channel_id;

-- recent errors on tnexus channel
SELECT created_at, type, model_name, use_time, other
FROM logs
WHERE channel_id = 114
  AND type != 2
ORDER BY created_at DESC
LIMIT 20;

-- multipart-related errors (both channels)
SELECT channel_id, created_at, model_name, other
FROM logs
WHERE channel_id IN (84, 114)
  AND created_at >= '2026-08-03 00:00:00'
  AND (other ILIKE '%multipart%' OR other ILIKE '%image field%')
ORDER BY created_at DESC
LIMIT 30;
SQL
