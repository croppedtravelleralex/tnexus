#!/usr/bin/env bash
# Does a dead account actually leave the rotation? Sweep a slice of the pool
# and show the health mix before and after.
set -u
set -a; . /opt/grokproxy/.env; set +a
base=http://127.0.0.1:8110
A=(-H "Authorization: Bearer $GROKPROXY_ADMIN_KEY")

show() {
  curl -s --max-time 20 "$base/api/v1/stats" "${A[@]}" | python3 -c '
import json,sys
d = json.load(sys.stdin)
for provider, mix in sorted(d.items()):
    parts = " ".join(f"{k}={v}" for k, v in sorted(mix.items()))
    print(f"  {provider:6} {parts}")'
}

echo "=== before ==="; show

echo
echo "=== sweeping 300 build accounts ==="
curl -s --max-time 900 -X POST "$base/api/v1/sweep?limit=300&concurrency=8" "${A[@]}" \
  | python3 -m json.tool

echo
echo "=== after ==="; show

echo
echo "=== what the pool would actually schedule now ==="
curl -s --max-time 30 "$base/api/v1/accounts?limit=5&health=active&provider=build" "${A[@]}" \
  > /tmp/sched.json
python3 - <<'PY'
import json
d = json.load(open('/tmp/sched.json'))
print(f"  schedulable build accounts: {d['total']}")
for a in d['accounts']:
    flag = "" if a['verified'] else "  (never proven)"
    print(f"    {a['email'][:36]:38} succ={a['success_count']} fail={a['failure_count']}{flag}")
PY
