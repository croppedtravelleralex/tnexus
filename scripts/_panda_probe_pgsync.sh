#!/usr/bin/env bash
set -u

echo "=== all tables ==="
docker exec panda-postgres-1 psql -U tnexus -d tnexus -t -c "select tablename from pg_tables where schemaname='public' order by 1;" 2>&1

echo
echo "=== tnexus_accounts freshness ==="
docker exec panda-postgres-1 psql -U tnexus -d tnexus -c "select count(*) as n, min(updated_at) as oldest, max(updated_at) as newest from tnexus_accounts;" 2>&1

echo
echo "=== tnexus_accounts columns ==="
docker exec panda-postgres-1 psql -U tnexus -d tnexus -c "\d tnexus_accounts" 2>&1 | head -40

echo
echo "=== account-ops routes (strings in binary) ==="
docker exec panda-account-ops-1 sh -c "strings /usr/local/bin/tnexus-account-ops 2>/dev/null | grep -E '^/(api|account|v1)/' | sort -u | head -40" 2>/dev/null \
  || docker exec panda-account-ops-1 sh -c "grep -aoE '/(api|accounts?)/[a-z0-9/_-]+' \$(command -v tnexus-account-ops) | sort -u | head -40"

echo
echo "=== TNexus repo: scripts that push accounts to panda/postgres ==="
grep -rln 'tnexus_accounts\|account_ops\|account-ops\|/api/accounts/import\|upsert_accounts' /root/TNexus/scripts/ /root/TNexus/deploy/ 2>/dev/null | head -20

echo
echo "=== deploy/panda dir ==="
ls -1 /root/TNexus/deploy/panda/ 2>/dev/null | head -60

echo
echo "=== any cron/systemd timer doing account sync ==="
crontab -l 2>/dev/null | head -30
echo "--- systemd timers ---"
systemctl list-timers --all 2>/dev/null | head -20
