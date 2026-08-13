#!/usr/bin/env bash
# Compare the verify_delivery placeholder prompt vs a real descriptive prompt.
set -uo pipefail

GW=$(grep -E '^GATEWAY_AUTH_KEY=' /opt/tnexus/.env | cut -d= -f2- | tr -d '\r\n')
URL=http://127.0.0.1:8014/v1/images/generations

run() {
  local label="$1" prompt="$2"
  local body out code
  body=$(python3 -c "
import json,sys
print(json.dumps({'model':'gpt-image-2','prompt':sys.argv[1],'n':1,'size':'256x256','response_format':'b64_json'}))
" "$prompt")
  out=$(mktemp)
  code=$(curl -sS -o "$out" -w '%{http_code}' --max-time 240 -X POST "$URL" \
    -H "Authorization: Bearer $GW" -H 'Content-Type: application/json' -d "$body")
  echo "--- $label -> http=$code"
  if [ "$code" = "200" ]; then
    python3 -c "
import json
d=json.load(open('$out'))
it=(d.get('data') or [{}])[0]
b=it.get('b64_json') or ''
print('   OK b64_len=%d url=%s' % (len(b), (it.get('url') or '')[:80]))
"
  else
    head -c 400 "$out"; echo
  fi
  rm -f "$out"
}

run "placeholder(verify_delivery)" "verify_delivery"
sleep 5
run "descriptive" "a red apple on a wooden table, simple product photo"
sleep 5
run "placeholder-retry" "verify_delivery"
