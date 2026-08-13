#!/usr/bin/env bash
set -u

echo "=== gateway logs since ETL (09:33 UTC) ==="
docker logs --since 20m panda-gateway-1 2>&1 | tail -60

echo
echo "=== error kind histogram last 30m ==="
docker logs --since 30m panda-gateway-1 2>&1 \
  | grep -oE 'error=[a-z_]+ HTTP [0-9]+|error=[a-z_ ]+' \
  | sort | uniq -c | sort -rn | head -20
