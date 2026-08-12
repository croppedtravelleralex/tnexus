#!/bin/bash
set -euo pipefail
# 深挖 TNexus-NewAPI (channel 115) gpt-image-2 失败明细
MODEL="gpt-image-2"
CH="${CH:-115}"
NOW=$(docker exec new-api-postgres psql -U newapi -d new-api -tAc "SELECT extract(epoch from now())::bigint;")
D1=$((NOW - 86400))
D7=$((NOW - 7*86400))

echo "=== logs 表结构 ==="
docker exec new-api-postgres psql -U newapi -d new-api -c "\d logs" 2>&1 | head -40

echo
echo "=== 近24h 失败样本完整 other JSON (最新5条) ==="
docker exec new-api-postgres psql -U newapi -d new-api -tAc "
SELECT other FROM logs
WHERE channel_id=${CH} AND model_name='${MODEL}' AND type!=2 AND created_at >= ${D1}
ORDER BY created_at DESC LIMIT 5;
"

echo
echo "=== 近24h 失败 content 字段 (错误文本) ==="
docker exec new-api-postgres psql -U newapi -d new-api -c "
SELECT LEFT(COALESCE(content,''),160) AS content, COUNT(*) AS n
FROM logs
WHERE channel_id=${CH} AND model_name='${MODEL}' AND type!=2 AND created_at >= ${D1}
GROUP BY 1 ORDER BY 2 DESC LIMIT 15;
"

echo
echo "=== 近7d 失败 content 分布 ==="
docker exec new-api-postgres psql -U newapi -d new-api -c "
SELECT LEFT(COALESCE(content,''),160) AS content, COUNT(*) AS n
FROM logs
WHERE channel_id=${CH} AND model_name='${MODEL}' AND type!=2 AND created_at >= ${D7}
GROUP BY 1 ORDER BY 2 DESC LIMIT 15;
"

echo
echo "=== 近24h 按小时成功率 ==="
docker exec new-api-postgres psql -U newapi -d new-api -c "
SELECT to_char(to_timestamp(created_at) AT TIME ZONE 'Asia/Shanghai','MM-DD HH24') AS hour,
       COUNT(*) AS req,
       SUM(CASE WHEN type=2 THEN 1 ELSE 0 END) AS ok,
       ROUND(100.0*SUM(CASE WHEN type=2 THEN 1 ELSE 0 END)/COUNT(*),1) AS ok_pct,
       ROUND(AVG(use_time)::numeric,1) AS avg_s
FROM logs
WHERE channel_id=${CH} AND model_name='${MODEL}' AND created_at >= ${D1}
GROUP BY 1 ORDER BY 1;
"

echo
echo "=== 失败请求 use_time 分布 (判断是否超时) ==="
docker exec new-api-postgres psql -U newapi -d new-api -c "
SELECT CASE
         WHEN use_time < 5 THEN 'a_lt5s'
         WHEN use_time < 30 THEN 'b_5-30s'
         WHEN use_time < 60 THEN 'c_30-60s'
         WHEN use_time < 120 THEN 'd_60-120s'
         WHEN use_time < 300 THEN 'e_120-300s'
         ELSE 'f_gt300s' END AS bucket,
       COUNT(*) AS n
FROM logs
WHERE channel_id=${CH} AND model_name='${MODEL}' AND type!=2 AND created_at >= ${D7}
GROUP BY 1 ORDER BY 1;
"

echo
echo "=== gateway 容器近24h 生图错误日志 ==="
docker logs panda-gateway-1 --since 24h 2>&1 | grep -iE 'image.*(fail|error|timeout|401|403|429|5[0-9][0-9])' | tail -25 || echo "(no match)"

echo
echo "=== NewAPI 容器近24h ch115 错误 ==="
docker logs new-api --since 24h 2>&1 | grep -iE 'channel.*115|tnexus-dedicated' | grep -iE 'fail|error|timeout|retry' | tail -20 || echo "(no match)"

echo
echo "=== channel 115 配置 ==="
docker exec new-api-postgres psql -U newapi -d new-api -c "
SELECT id, name, status, weight, priority, base_url,
       LEFT(COALESCE(model_mapping,''),80) AS model_mapping,
       response_time, test_time
FROM channels WHERE id=${CH};
"

echo
echo "=== gateway JWT 有效性 ==="
GW_KEY=$(grep -E '^GATEWAY_AUTH_KEY=' /opt/tnexus/.env | cut -d= -f2- | tr -d '\r\n')
CH_KEY=$(docker exec new-api-postgres psql -U newapi -d new-api -tAc "SELECT key FROM channels WHERE id=${CH}" | tr -d '\r\n')
if [ "$GW_KEY" = "$CH_KEY" ]; then echo "PASS key_match"; else echo "FAIL key_mismatch gw_len=${#GW_KEY} ch_len=${#CH_KEY}"; fi
python3 - "$GW_KEY" <<'PY'
import base64, json, sys, time
tok = sys.argv[1]
parts = tok.split(".")
if len(parts) != 3:
    print("not_a_jwt len=", len(tok)); raise SystemExit
p = parts[1] + "=" * (-len(parts[1]) % 4)
c = json.loads(base64.urlsafe_b64decode(p))
exp = c.get("exp")
now = int(time.time())
print(f"jwt exp={exp} now={now} valid_for_s={exp-now if exp else 'n/a'}")
PY
