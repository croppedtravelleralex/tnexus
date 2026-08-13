#!/usr/bin/env bash
# Locate the runtime that hosts services/account_service.py
set -u

echo "=== container probe ==="
for c in $(docker ps --format '{{.Names}}'); do
  out=$(docker exec "$c" sh -c 'ls -1 /app/services/ 2>/dev/null | head -40' 2>/dev/null)
  if [ -n "$out" ]; then
    echo "--- $c : /app/services ---"
    echo "$out"
  fi
  acc=$(docker exec "$c" sh -c 'ls -1 /app/services/account_service.py 2>/dev/null' 2>/dev/null)
  if [ -n "$acc" ]; then
    echo "*** HIT: $c has /app/services/account_service.py"
    docker exec "$c" sh -c 'command -v python3 || command -v python || echo NO_PYTHON' 2>/dev/null
  fi
done

echo
echo "=== container-wide find (account_service.py anywhere) ==="
for c in $(docker ps --format '{{.Names}}'); do
  hits=$(docker exec "$c" sh -c "find / -name 'account_service.py' -not -path '*/node_modules/*' 2>/dev/null | head -5" 2>/dev/null)
  if [ -n "$hits" ]; then
    echo "--- $c ---"
    echo "$hits"
  fi
done

echo
echo "=== host find ==="
find / -name 'account_service.py' -not -path '*/node_modules/*' -not -path '/proc/*' -not -path '/sys/*' 2>/dev/null | head -20

echo
echo "=== host find account_refresh_all_service ==="
find / -name 'account_refresh_all_service*' -not -path '*/node_modules/*' -not -path '/proc/*' -not -path '/sys/*' 2>/dev/null | head -20
