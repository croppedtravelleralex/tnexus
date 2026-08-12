#!/usr/bin/env bash
# 只统计 gateway 当前容器启动之后的生图请求——混入部署前的记录会让修复效果无法判断。
set -uo pipefail
MODEL="gpt-image-2"
CH="${CH:-115}"

DEPLOY_ISO=$(docker inspect -f '{{.State.StartedAt}}' panda-gateway-1)
DEPLOY_TS=$(date -d "$DEPLOY_ISO" +%s)
NOW=$(date +%s)
echo "=== gateway StartedAt=$DEPLOY_ISO (epoch=$DEPLOY_TS, $(( (NOW - DEPLOY_TS) / 60 )) 分钟前) ==="

echo
echo "=== ch${CH} since deploy (全部调用方) ==="
docker exec new-api-postgres psql -U newapi -d new-api -c "
SELECT COUNT(*) AS req,
       SUM(CASE WHEN type=2 THEN 1 ELSE 0 END) AS ok,
       SUM(CASE WHEN type!=2 THEN 1 ELSE 0 END) AS fail,
       ROUND(100.0*SUM(CASE WHEN type=2 THEN 1 ELSE 0 END)/NULLIF(COUNT(*),0),2) AS ok_pct,
       ROUND((PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY use_time))::numeric,1) AS p50_s,
       ROUND((PERCENTILE_CONT(0.95) WITHIN GROUP (ORDER BY use_time))::numeric,1) AS p95_s,
       ROUND((PERCENTILE_CONT(0.99) WITHIN GROUP (ORDER BY use_time))::numeric,1) AS p99_s
FROM logs
WHERE model_name='${MODEL}' AND channel_id=${CH} AND created_at >= ${DEPLOY_TS};
"

echo "=== ch${CH} since deploy (真实用户，排除 root 渠道自测) ==="
docker exec new-api-postgres psql -U newapi -d new-api -c "
SELECT COUNT(*) AS req,
       SUM(CASE WHEN type=2 THEN 1 ELSE 0 END) AS ok,
       ROUND(100.0*SUM(CASE WHEN type=2 THEN 1 ELSE 0 END)/NULLIF(COUNT(*),0),2) AS ok_pct
FROM logs
WHERE model_name='${MODEL}' AND channel_id=${CH}
  AND username <> 'root' AND created_at >= ${DEPLOY_TS};
"

echo "=== since deploy 失败明细 ==="
docker exec new-api-postgres psql -U newapi -d new-api -c "
SELECT LEFT(COALESCE(content,''),150) AS content, username, COUNT(*) AS n
FROM logs
WHERE model_name='${MODEL}' AND channel_id=${CH} AND type!=2 AND created_at >= ${DEPLOY_TS}
GROUP BY 1,2 ORDER BY 3 DESC LIMIT 10;
"

echo "=== 对照：部署前 24h ==="
docker exec new-api-postgres psql -U newapi -d new-api -c "
SELECT COUNT(*) AS req,
       SUM(CASE WHEN type=2 THEN 1 ELSE 0 END) AS ok,
       ROUND(100.0*SUM(CASE WHEN type=2 THEN 1 ELSE 0 END)/NULLIF(COUNT(*),0),2) AS ok_pct
FROM logs
WHERE model_name='${MODEL}' AND channel_id=${CH}
  AND created_at >= ${DEPLOY_TS} - 86400 AND created_at < ${DEPLOY_TS};
"

echo "=== gateway 日志 since deploy：错误类型分布 ==="
docker logs panda-gateway-1 --since "$DEPLOY_ISO" 2>&1 \
  | sed 's/\x1b\[[0-9;]*m//g' \
  | grep -oE 'error=[a-z_]+ HTTP [0-9]+' | sort | uniq -c | sort -rn | head -10
echo "(无输出 = 期间无上游错误)"
