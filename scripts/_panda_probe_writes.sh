#!/usr/bin/env bash
set -u

echo "=== Rust writes to tnexus_accounts ==="
grep -rn 'tnexus_accounts' /root/TNexus/crates/ --include=*.rs | head -30

echo
echo "=== UPDATE/INSERT statements on tnexus_accounts ==="
grep -rn -B3 -A 12 'UPDATE tnexus_accounts\|INSERT INTO tnexus_accounts' /root/TNexus/crates/ --include=*.rs | head -60

echo
echo "=== dead / flagged account status ==="
docker exec panda-postgres-1 psql -U tnexus -d tnexus -c \
"select email, data->>'status' as status, left(coalesce(data->>'last_token_refresh_error',''),40) as err
 from tnexus_accounts
 where email in ('agustinkelly59361@outlook.com','qaflowfbdb3ovksr@proton.me','dorishunk41971@outlook.com');"

echo
echo "=== scheduling_state.json (gateway manual gate) ==="
python3 -c "
import json
p='/opt/tnexus/data/pool/scheduling_state.json'
try:
    d=json.load(open(p,encoding='utf-8'))
except Exception as e:
    print('ERR',e); raise SystemExit
if isinstance(d,dict):
    print('entries=%d' % len(d))
    for k,v in list(d.items())[:40]:
        print('  %-40s %s' % (k,v))
else:
    print(type(d), str(d)[:300])
"
