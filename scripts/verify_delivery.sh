#!/usr/bin/env bash
# End-to-end delivery verification (run on Panda or via ssh panda).
set -euo pipefail

ENV_FILE="${ENV_FILE:-/opt/tnexus/.env}"
FAIL=0

pass() { echo "PASS $*"; }
fail() { echo "FAIL $*"; FAIL=1; }

# shellcheck disable=SC1090
source "$ENV_FILE"

echo "=== TNexus health ==="
curl -fsS http://127.0.0.1:9000/health >/dev/null && pass tnexus_health || fail tnexus_health

echo "=== Grok ready + pagination ==="
curl -fsS http://127.0.0.1:8000/readyz >/dev/null && pass grok_readyz || fail grok_readyz
ADMIN_PW="${GROK_ADMIN_PASSWORD:?GROK_ADMIN_PASSWORD missing}"
TOK=$(curl -fsS -H "Content-Type: application/json" \
  -d "{\"username\":\"admin\",\"password\":\"$ADMIN_PW\"}" \
  http://127.0.0.1:8091/admin/auth/login \
  | python3 -c "import sys,json; print(json.load(sys.stdin)['tokens']['access_token'])")
PAGE=$(curl -fsS -H "Authorization: Bearer $TOK" \
  "http://127.0.0.1:8091/admin/accounts?page=2&pageSize=200")
echo "$PAGE" | python3 -c "
import sys,json
d=json.load(sys.stdin)
n=len(d.get('items',[]))
p=d.get('page')
s=d.get('pageSize')
assert n==200 and p==2 and s==200, (n,p,s)
print('ok',n,p,s)
" && pass grok_pagination || fail grok_pagination

echo "=== Grok nurture + quota ==="
curl -fsS -H "Authorization: Bearer $TOK" http://127.0.0.1:8091/admin/nurture/status \
  | python3 -c "import sys,json; d=json.load(sys.stdin); assert d.get('running') is True" \
  && pass grok_nurture || fail grok_nurture
curl -fsS -X POST -H "Authorization: Bearer $TOK" -H "Content-Type: application/json" \
  -d '{"limit":2}' http://127.0.0.1:8091/admin/accounts/web/refresh-quotas \
  | python3 -c "import sys,json; d=json.load(sys.stdin); assert d.get('ok',0)>=0" \
  && pass grok_quota_refresh || fail grok_quota_refresh

echo "=== GPT nurture ==="
BOOT_EMAIL="${BOOTSTRAP_ADMIN_EMAIL:?}"
BOOT_PASS="${BOOTSTRAP_ADMIN_PASSWORD:?}"
SESS=$(curl -fsS -c - -X POST http://127.0.0.1:9000/api/auth/login \
  -H "Content-Type: application/json" \
  -d "{\"email\":\"$BOOT_EMAIL\",\"password\":\"$BOOT_PASS\"}" \
  | awk '/tnexus_session/ {print $NF}')
curl -fsS "http://127.0.0.1:9000/api/ops/nurture/status" \
  -H "Cookie: tnexus_session=$SESS" \
  | python3 -c "import sys,json; d=json.load(sys.stdin); assert d.get('running') and d.get('worker_alive')" \
  && pass gpt_nurture || fail gpt_nurture

echo "=== Gateway image + JWT ==="
GW="${GATEWAY_AUTH_KEY:?}"
python3 -c "import jwt,sys,time; t=sys.argv[1]; p=jwt.decode(t,options={'verify_signature':False}); assert p.get('exp',0)>int(time.time())" "$GW" \
  && pass jwt_valid || fail jwt_valid
CODE=$(curl -sS -o /dev/null -w '%{http_code}' -X POST http://127.0.0.1:8014/v1/images/generations \
  -H "Authorization: Bearer $GW" -H "Content-Type: application/json" \
  -d '{"model":"gpt-image-2","prompt":"verify_delivery","n":1,"size":"256x256","response_format":"b64_json"}')
[[ "$CODE" == "200" ]] && pass gateway_image || fail "gateway_image http=$CODE"

echo "=== NewAPI channel key sync ==="
CHKEY=$(docker exec new-api-postgres psql -U newapi -d new-api -tAc "SELECT key FROM channels WHERE id=115" | tr -d '\n')
[[ "$GW" == "$CHKEY" ]] && pass newapi_channel_key || fail newapi_channel_key

echo "=== Grok keys sync ==="
KEYS_DIR="${GROK_PURE_HTTP_KEYS_DIR:-/opt/tnexus/pure_http_keys}"
NKEYS=$(find "$KEYS_DIR" -maxdepth 1 -name 'account_*.json' 2>/dev/null | wc -l)
[[ "$NKEYS" -ge 1 ]] && pass "grok_keys_present count=$NKEYS" || fail grok_keys_present
export GROK_DATABASE_URL
bash /root/TNexus/scripts/sync_grok_enabled_from_keys.sh --keys-dir "$KEYS_DIR" --dry-run >/dev/null \
  && pass grok_keys_sync_dryrun || fail grok_keys_sync_dryrun

if [[ "$FAIL" -eq 0 ]]; then
  echo "=== ALL PASS ==="
  exit 0
fi
echo "=== SOME CHECKS FAILED ==="
exit 1
