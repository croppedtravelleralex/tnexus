#!/usr/bin/env bash
# 诊断 gptimage-gateway 账号池 access_token 过期情况
set -uo pipefail

K=$(grep -E '^GATEWAY_AUTH_KEY=' /opt/tnexus/.env | cut -d= -f2- | tr -d '\r\n')
GW=http://127.0.0.1:8014
# docker logs 带 ANSI 颜色，先剥离再匹配
STRIP='s/\x1b\[[0-9;]*m//g'

echo "=== /api/accounts 概览 ==="
curl -sS --max-time 15 -H "Authorization: Bearer $K" "$GW/api/accounts" 2>/dev/null \
  | python3 -c '
import json,sys
try:
    d = json.load(sys.stdin)
except Exception as e:
    print("parse_error", e); raise SystemExit
items = d if isinstance(d, list) else (d.get("accounts") or d.get("data") or d.get("items") or [])
print("total_accounts:", len(items))
if items:
    print("sample_keys:", sorted(items[0].keys()))
from collections import Counter
for f in ("status","enabled","disabled","state","token_status","healthy"):
    vals = [it.get(f) for it in items if isinstance(it, dict) and f in it]
    if vals:
        print(f"{f}:", dict(Counter(map(str, vals))))
'

echo
echo "=== 近24h 各账号 image 尝试次数 ==="
docker logs panda-gateway-1 --since 24h 2>&1 | sed "$STRIP" \
  | grep -oE 'email=[^ ]+' | sort | uniq -c | sort -rn | head -25

echo
echo "=== 近24h token_expired 涉及账号 ==="
docker logs panda-gateway-1 --since 24h 2>&1 | sed "$STRIP" \
  | grep -E 'upstream image failed|image call failed' \
  | grep -oE 'email=[^ ]+' | sort | uniq -c | sort -rn | head -25

echo
echo "=== 近24h image ok 账号 ==="
docker logs panda-gateway-1 --since 24h 2>&1 | sed "$STRIP" \
  | grep 'image ok' | grep -oE 'email=[^ ]+' | sort | uniq -c | sort -rn | head -25

echo
echo "=== 近24h 错误类型分布 ==="
docker logs panda-gateway-1 --since 24h 2>&1 | sed "$STRIP" \
  | grep -oE 'error=[a-z_]+ HTTP [0-9]+' | sort | uniq -c | sort -rn | head -15

echo
echo "=== accounts_pool.json 账号数 ==="
python3 -c '
import json
d = json.load(open("/root/gptimage-gateway-rs/secrets/accounts_pool.json"))
items = d if isinstance(d, list) else (d.get("accounts") or [])
print("pool_size:", len(items))
if items:
    print("keys:", sorted(items[0].keys()))
'
