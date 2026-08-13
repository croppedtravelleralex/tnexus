#!/usr/bin/env bash
set -uo pipefail

echo "=== gateway pool summary ==="
bash /root/TNexus/deploy/panda/gpt_pool_refresh_detail.sh 2>/dev/null | tail -8

echo
echo "=== upstream vs gateway drift ==="
bash /tmp/_panda_pool_diff.sh 2>/dev/null | tail -4

echo
echo "=== gateway errors in last 20m (post-fix) ==="
N401=$(docker logs --since 20m panda-gateway-1 2>&1 | grep -c 'HTTP 401' || true)
NERR=$(docker logs --since 20m panda-gateway-1 2>&1 | grep -c 'image call failed' || true)
echo "http_401=$N401 image_call_failed=$NERR"

echo
echo "=== 401 count before vs after ETL (ETL ran 09:33 UTC) ==="
echo -n "last 24h total 401: "; docker logs --since 24h panda-gateway-1 2>&1 | grep -c 'chat_requirements_prepare HTTP 401' || true
echo -n "since ETL   401: "; docker logs --since 25m panda-gateway-1 2>&1 | grep -c 'chat_requirements_prepare HTTP 401' || true

echo
echo "=== cron installed ==="
ls -la /etc/cron.d/tnexus-accounts-sync && cat /etc/cron.d/tnexus-accounts-sync
