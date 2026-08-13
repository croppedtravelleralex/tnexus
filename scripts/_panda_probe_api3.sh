#!/usr/bin/env bash
set -u
C=chatgpt2api-local

echo "=== AccountRefreshRequest model ==="
docker exec "$C" sh -c "grep -rn -A 12 'class AccountRefreshRequest' /app/api/ 2>/dev/null | head -30"

echo
echo "=== /api/accounts/refresh handler ==="
docker exec "$C" sh -c "sed -n '705,760p' /app/api/accounts.py"

echo
echo "=== auth dependency (require_auth / verify) ==="
docker exec "$C" sh -c "grep -rn 'def require_auth\|def _require\|def verify_auth\|_check_auth' /app/api/*.py /app/services/auth_service.py 2>/dev/null | head -20"

echo
echo "=== proactive refresh status impl (why did it not fire?) ==="
docker exec "$C" sh -c "sed -n '815,830p' /app/api/accounts.py"

echo
echo "=== proactive refresh service ==="
docker exec "$C" sh -c "ls -1 /app/services/ | grep -i 'proactive\|maintenance\|refresh'"
