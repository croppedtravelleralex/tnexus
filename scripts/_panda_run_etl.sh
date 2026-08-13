#!/usr/bin/env bash
# Push fresh access_tokens from gptimage sqlite -> Postgres tnexus_accounts (gateway source).
set -uo pipefail

STAMP=$(date +%Y%m%d-%H%M%S)
BK=/root/backups/tnexus_accounts-$STAMP.sql
mkdir -p /root/backups

echo "=== verify ETL script integrity ==="
grep -c 'INSERT INTO tnexus_accounts' /root/TNexus/scripts/etl_accounts_to_postgres.py
grep -n 'ON CONFLICT' /root/TNexus/scripts/etl_accounts_to_postgres.py

echo
echo "=== backup tnexus_accounts -> $BK ==="
docker exec panda-postgres-1 pg_dump -U tnexus -d tnexus -t tnexus_accounts --data-only > "$BK" 2>/dev/null
echo "backup_bytes=$(wc -c < "$BK")"

echo
echo "=== pre-ETL state ==="
docker exec panda-postgres-1 psql -U tnexus -d tnexus -t -c "select count(*) from tnexus_accounts;"

echo
echo "=== running ETL ==="
cd /root/TNexus
ACCOUNTS_DB=/root/gptimage/data/accounts.db \
DATABASE_URL='postgres://tnexus:914c7b5f0b459509cac9a474f9e8868e@127.0.0.1:5433/tnexus' \
python3 scripts/etl_accounts_to_postgres.py
RC=$?
echo "etl_rc=$RC"

echo
echo "=== reconcile ==="
ACCOUNTS_DB=/root/gptimage/data/accounts.db \
DATABASE_URL='postgres://tnexus:914c7b5f0b459509cac9a474f9e8868e@127.0.0.1:5433/tnexus' \
python3 scripts/reconcile_accounts_postgres.py

echo
echo "=== post-ETL updated_at ==="
docker exec panda-postgres-1 psql -U tnexus -d tnexus -c "select count(*) n, min(updated_at) oldest, max(updated_at) newest from tnexus_accounts;"

echo
echo "BACKUP_PATH=$BK"
