#!/usr/bin/env bash
# Prove the quota pipeline end to end on the live box: fire one chat through
# grokProxy, then read back what the account recorded.
set -u
set -a; . /opt/grokproxy/.env; set +a
base=http://127.0.0.1:8110

echo "=== janitor + listener ==="
docker logs grokproxy 2>&1 | grep -iE 'janitor|listening' | tail -3

echo
echo "=== one chat through grokproxy ==="
curl -s --max-time 120 -X POST "$base/v1/chat/completions" \
  -H "Authorization: Bearer $GROKPROXY_API_KEY" \
  -H 'Content-Type: application/json' \
  -d '{"model":"grok-4.6","messages":[{"role":"user","content":"say ok"}],"max_tokens":8}' \
  | head -c 260
echo

echo
echo "=== accounts that have reported quota ==="
curl -s --max-time 30 "$base/api/v1/accounts?limit=200" \
  -H "Authorization: Bearer $GROKPROXY_ADMIN_KEY" > /tmp/acc.json
python3 - <<'PY'
import json
d = json.load(open('/tmp/acc.json'))
rows = d['accounts']
known = [a for a in rows if a.get('remaining_tokens') is not None]
print(f"total={d['total']}  page={len(rows)}  with quota data={len(known)}")
for a in sorted(known, key=lambda x: -x['success_count'])[:8]:
    print(f"  {a['email'][:32]:34} {a['health']:<6} "
          f"tok {a['remaining_tokens']:>9,}/{a['limit_tokens']:<9,} "
          f"req {a['remaining_requests']:>3}/{a['limit_requests']:<3} "
          f"succ={a['success_count']}")
if not known:
    print("  (none yet — quota is only learned when an account serves a request)")
PY

echo
echo "=== health mix ==="
curl -s --max-time 20 "$base/api/v1/stats" -H "Authorization: Bearer $GROKPROXY_ADMIN_KEY" \
  | python3 -m json.tool
