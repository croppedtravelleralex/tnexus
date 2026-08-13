#!/usr/bin/env bash
set -u
C=chatgpt2api-local

echo "=== panda_sync config (values masked) ==="
python3 - <<'PY'
import json
d = json.load(open('/root/gptimage/config.json', encoding='utf-8'))
ps = d.get('panda_sync') or {}
for k, v in ps.items():
    s = str(v)
    if any(x in k.lower() for x in ('key','token','secret','password')):
        s = s[:6] + '...len=%d' % len(str(v))
    print(f'  {k} = {s}')
PY

echo
echo "=== queue_refreshed_tokens_for_panda impl ==="
docker exec "$C" sh -c "grep -n -A 40 'def queue_refreshed_tokens_for_panda' /app/services/account_refresh_all_service.py | head -60"

echo
echo "=== panda sync worker / push functions ==="
docker exec "$C" sh -c "grep -n 'def .*panda' /app/services/account_refresh_all_service.py | head -30"

echo
echo "=== other files referencing panda sync ==="
docker exec "$C" sh -c "grep -rln 'panda_sync\|panda-sync\|sync_to_panda' /app/services/ /app/api/ 2>/dev/null | head -20"

echo
echo "=== recent chatgpt2api-local logs mentioning panda/sync ==="
docker logs --since 24h chatgpt2api-local 2>&1 | grep -iE 'panda|sync' | tail -40
