#!/usr/bin/env bash
# Does the free Build upstream expose a quota/credits endpoint, or is per-token
# accounting the only thing available? Determines whether "remaining tokens"
# can be shown at all, or only "tokens used".
set -u
BASE="https://cli-chat-proxy.grok.com/v1"
token="${ACCESS_TOKEN:?ACCESS_TOKEN required}"
proxy="${EGRESS_PROXY:-}"

curl_args=(-s -o /tmp/q.out -w '%{http_code}' --max-time 20
  -H "Authorization: Bearer ${token}"
  -H 'X-XAI-Token-Auth: xai-grok-cli'
  -H 'x-grok-client-version: 0.2.93'
  -H 'x-grok-client-identifier: grok-shell')
[[ -n "$proxy" ]] && curl_args+=(-x "$proxy")

for path in /models /usage /credits /quota /billing/usage /me /account /subscriptions /rate_limits; do
  code="$(curl "${curl_args[@]}" "${BASE}${path}" || echo 000)"
  body="$(head -c 220 /tmp/q.out 2>/dev/null | tr -d '\n')"
  printf '%-18s %s  %s\n' "$path" "$code" "$body"
done

echo
echo "--- what a chat response reports as usage ---"
curl -s --max-time 60 "${BASE}/chat/completions" \
  ${proxy:+-x "$proxy"} \
  -H "Authorization: Bearer ${token}" \
  -H 'Content-Type: application/json' \
  -H 'X-XAI-Token-Auth: xai-grok-cli' \
  -H 'x-grok-client-version: 0.2.93' \
  -H 'x-grok-client-identifier: grok-shell' \
  -d '{"model":"grok-4.6","messages":[{"role":"user","content":"hi"}],"max_tokens":4,"stream":false}' \
  | python3 -c 'import json,sys; d=json.load(sys.stdin); print(json.dumps(d.get("usage",{}), indent=2)[:800])' 2>&1
