#!/usr/bin/env bash
# Inspect account_service / account_refresh_all_service API surface in chatgpt2api-local
set -u
C=chatgpt2api-local

echo "=== def list_tokens / get_account / refresh_accounts in account_service.py ==="
docker exec "$C" sh -c "grep -n 'def list_tokens\|def get_account\|def refresh_accounts\|def refresh_account\b\|^account_service' /app/services/account_service.py | head -40"

echo
echo "=== refresh_accounts signature context ==="
docker exec "$C" sh -c "grep -n -A 12 'def refresh_accounts' /app/services/account_service.py | head -60"

echo
echo "=== sync_last_refreshed_accounts_to_panda ==="
docker exec "$C" sh -c "grep -n -A 25 'def sync_last_refreshed_accounts_to_panda' /app/services/account_refresh_all_service.py | head -80"

echo
echo "=== module singletons ==="
docker exec "$C" sh -c "grep -n '^account_refresh_all_service\|^account_service' /app/services/account_refresh_all_service.py /app/services/account_service.py | head -20"
