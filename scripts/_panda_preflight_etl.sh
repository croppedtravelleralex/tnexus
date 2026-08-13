#!/usr/bin/env bash
set -u

echo "=== tnexus_account_runtime schema (does runtime state live here?) ==="
docker exec panda-postgres-1 psql -U tnexus -d tnexus -c "\d tnexus_account_runtime" 2>&1 | head -30

echo
echo "=== runtime rows sample ==="
docker exec panda-postgres-1 psql -U tnexus -d tnexus -c "select email, image_schedulable, image_fail_streak from tnexus_account_runtime order by image_schedulable, email limit 30;" 2>&1 | head -40

echo
echo "=== tnexus_accounts.data keys (is image_schedulable stored here too?) ==="
docker exec panda-postgres-1 psql -U tnexus -d tnexus -t -c "select jsonb_object_keys(data) from tnexus_accounts limit 200;" 2>&1 | sort -u | tr '\n' ' ' | head -c 2000
echo

echo
echo "=== sqlite accounts schema ==="
python3 -c "
import sqlite3
c=sqlite3.connect('file:/root/gptimage/data/accounts.db?mode=ro',uri=True)
print([r[0] for r in c.execute(\"select name from sqlite_master where type='table'\")][:20])
print(c.execute('select count(*) from accounts').fetchone())
print([d[0] for d in c.execute('select * from accounts limit 1').description])
"

echo
echo "=== psycopg2 availability on host ==="
python3 -c "import psycopg2; print('psycopg2 OK', psycopg2.__version__)" 2>&1

echo
echo "=== gateway account caching: does it re-read pg per request? (check restart need) ==="
grep -rn 'ACCOUNTS_BACKEND\|accounts_backend' /root/TNexus/crates/*/src/*.rs 2>/dev/null | head -10

echo
echo "=== last 5 tnexus_accounts updated_at ==="
docker exec panda-postgres-1 psql -U tnexus -d tnexus -c "select email, updated_at from tnexus_accounts order by updated_at desc limit 5;" 2>&1
