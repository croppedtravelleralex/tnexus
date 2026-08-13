#!/usr/bin/env bash
set -u
C=chatgpt2api-local

echo "=== require_admin impl ==="
docker exec "$C" sh -c "grep -rn -A 20 'def require_admin' /app/api/*.py /app/services/*.py 2>/dev/null | head -40"

echo
echo "=== auth_service token source ==="
docker exec "$C" sh -c "grep -rn 'AUTH_KEY\|auth_key\|admin_key\|ADMIN' /app/services/auth_service.py 2>/dev/null | head -30"

echo
echo "=== config.json auth-ish keys (values masked) ==="
python3 - <<'PY'
import json
try:
    d = json.load(open('/root/gptimage/config.json', encoding='utf-8'))
except Exception as e:
    print('ERR', e); raise SystemExit(0)

def walk(o, p=''):
    if isinstance(o, dict):
        for k, v in o.items():
            np = f'{p}.{k}' if p else k
            if isinstance(v, (dict, list)):
                walk(v, np)
            else:
                if any(s in k.lower() for s in ('key', 'token', 'auth', 'secret', 'password')):
                    s = str(v)
                    print(f'{np} = {s[:6]}...len={len(s)}')
walk(d)
PY

echo
echo "=== container env auth ==="
docker exec "$C" sh -c "env | grep -iE 'auth|key|secret' | sed -E 's/=(.{0,6}).*/=\1...(masked)/'"
