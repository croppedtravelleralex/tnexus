#!/usr/bin/env bash
set -u
C=chatgpt2api-local

echo "=== /app/api listing ==="
docker exec "$C" sh -c "ls -1 /app/api/ 2>/dev/null | head -60"

echo
echo "=== endpoints referencing refresh_accounts ==="
docker exec "$C" sh -c "grep -rn 'refresh_accounts\|sync_last_refreshed_accounts_to_panda' /app/api/ 2>/dev/null | head -30"

echo
echo "=== route decorators containing 'refresh' ==="
docker exec "$C" sh -c "grep -rn -B2 -A2 \"refresh\" /app/api/*.py 2>/dev/null | grep -E 'router\.(post|get)|@app\.(post|get)' | head -40"

echo
echo "=== auth scheme for api ==="
docker exec "$C" sh -c "grep -rn 'AUTH_TOKEN\|API_KEY\|Authorization' /app/api/*.py 2>/dev/null | head -20"

echo
echo "=== in-memory caching in AccountService (does it re-read sqlite?) ==="
docker exec "$C" sh -c "grep -n '_accounts_cache\|self._cache\|_reload\|def _load' /app/services/account_service.py | head -30"

echo
echo "=== sqlite journal mode / wal ==="
ls -la /root/gptimage/data/accounts.db* 2>/dev/null
