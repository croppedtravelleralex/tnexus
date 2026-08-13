#!/usr/bin/env bash
# The upstream's x-ratelimit-* headers are static (verified: 6 back-to-back
# calls never moved them), so they advertise entitlement, not remaining. The
# only true consumption counter is the `usage` block we sum locally.
# This checks that local accounting actually tracks what we spend.
set -u
set -a; . /opt/grokproxy/.env; set +a
base=http://127.0.0.1:8110

snapshot() {
  curl -s --max-time 30 "$base/api/v1/accounts?limit=400" \
    -H "Authorization: Bearer $GROKPROXY_ADMIN_KEY"
}

snapshot > /tmp/before.json

echo "=== firing 3 chats through grokproxy ==="
for i in 1 2 3; do
  out="$(curl -s --max-time 120 -X POST "$base/v1/chat/completions" \
    -H "Authorization: Bearer $GROKPROXY_API_KEY" -H 'Content-Type: application/json' \
    -d '{"model":"grok-4.6","messages":[{"role":"user","content":"count to three"}],"max_tokens":40}')"
  echo "  #$i usage: $(printf '%s' "$out" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("usage"))' 2>/dev/null || echo 'no usage block')"
done

sleep 1
snapshot > /tmp/after.json

echo
echo "=== per-account token delta recorded locally ==="
python3 - <<'PY'
import json
before = {a['id']: a for a in json.load(open('/tmp/before.json'))['accounts']}
after = json.load(open('/tmp/after.json'))['accounts']
moved = 0
for a in after:
    b = before.get(a['id'])
    if not b:
        continue
    d_tok = a['total_tokens'] - b['total_tokens']
    d_ok = a['success_count'] - b['success_count']
    if d_tok or d_ok:
        moved += 1
        limit = a.get('limit_tokens')
        gauge = f"{a['total_tokens']:,}/{limit:,}" if limit else f"{a['total_tokens']:,}/?"
        print(f"  {a['email'][:32]:34} +{d_tok:>6,} tok  +{d_ok} req   used {gauge}")
if not moved:
    print("  NOTHING MOVED — local usage accounting is not recording")
PY
