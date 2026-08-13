#!/usr/bin/env bash
# Understand chatgpt2api-local storage backend + admin HTTP surface before mutating state
set -u
C=chatgpt2api-local

echo "=== storage backend config ==="
docker exec "$C" sh -c "grep -rn 'def get_storage_backend' -A 15 /app/services/config.py | head -40"

echo
echo "=== env hints ==="
docker exec "$C" sh -c "env | grep -iE 'storage|backend|db|postgres|redis|data_dir' | sed 's/\(PASSWORD=\).*/\1***/I' | head -20"

echo
echo "=== container mounts ==="
docker inspect chatgpt2api-local --format '{{range .Mounts}}{{.Source}} -> {{.Destination}} ({{.Mode}}){{println}}{{end}}'

echo
echo "=== published ports ==="
docker inspect chatgpt2api-local --format '{{json .NetworkSettings.Ports}}'

echo
echo "=== admin routes mentioning refresh ==="
docker exec "$C" sh -c "grep -rn 'refresh-accounts\|refresh_accounts\|/api/accounts/refresh' /app/routes/ /app/app.py /app/main.py 2>/dev/null | head -30"

echo
echo "=== routes dir ==="
docker exec "$C" sh -c "ls -1 /app/routes/ 2>/dev/null | head -40; echo '--- top-level ---'; ls -1 /app/*.py 2>/dev/null | head -20"
