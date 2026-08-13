#!/usr/bin/env bash
# Characterise the Build quota window: hammer one account and watch the
# x-ratelimit-* counters move. Capacity planning depends on knowing whether
# "21 requests" is per hour, per day, or per lifetime.
set -u
token="${ACCESS_TOKEN:?ACCESS_TOKEN required}"
H=(-H "Authorization: Bearer ${token}"
   -H 'X-XAI-Token-Auth: xai-grok-cli'
   -H 'x-grok-client-version: 0.2.93'
   -H 'x-grok-client-identifier: grok-shell'
   -H 'Content-Type: application/json')
body='{"model":"grok-4.6","messages":[{"role":"user","content":"hi"}],"max_tokens":4,"stream":false}'

printf '%-4s %-12s %-14s %-10s %-10s %s\n' '#' 'rem_req' 'rem_tokens' 'reset_req' 'reset_tok' 'status'
for i in $(seq 1 6); do
  hdr="$(curl -s -D - -o /tmp/q.out -w '%{http_code}' --max-time 90 \
        "${H[@]}" -d "$body" https://cli-chat-proxy.grok.com/v1/chat/completions)"
  code="${hdr##*$'\n'}"
  get() { printf '%s' "$hdr" | grep -i "^$1:" | tail -1 | cut -d' ' -f2- | tr -d '\r'; }
  printf '%-4s %-12s %-14s %-10s %-10s %s\n' \
    "$i" \
    "$(get x-ratelimit-remaining-requests)" \
    "$(get x-ratelimit-remaining-tokens)" \
    "$(get x-ratelimit-reset-requests)" \
    "$(get x-ratelimit-reset-tokens)" \
    "$code"
  sleep 1
done

echo
echo "=== every x-ratelimit / reset header seen on the last call ==="
printf '%s' "$hdr" | grep -iE 'ratelimit|reset|retry-after' | tr -d '\r'
