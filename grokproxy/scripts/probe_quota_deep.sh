#!/usr/bin/env bash
# Deeper hunt for Build quota: response headers, the full /models body, and the
# rate-limit endpoints the Grok web/CLI clients are known to use.
set -u
token="${ACCESS_TOKEN:?ACCESS_TOKEN required}"
H=(-H "Authorization: Bearer ${token}"
   -H 'X-XAI-Token-Auth: xai-grok-cli'
   -H 'x-grok-client-version: 0.2.93'
   -H 'x-grok-client-identifier: grok-shell'
   -H 'User-Agent: grok-cli/0.2.93')

echo "=== 1. response headers on /models (rate-limit hints?) ==="
curl -s -D - -o /dev/null --max-time 20 "${H[@]}" \
  https://cli-chat-proxy.grok.com/v1/models \
  | grep -iE 'ratelimit|quota|remaining|limit|reset|x-' | head -20
echo "(empty above = no rate-limit headers)"

echo
echo "=== 2. full /models body ==="
curl -s --max-time 20 "${H[@]}" https://cli-chat-proxy.grok.com/v1/models \
  | python3 -m json.tool 2>/dev/null | head -40

echo
echo "=== 3. headers on an actual chat call ==="
curl -s -D - -o /dev/null --max-time 60 "${H[@]}" \
  -H 'Content-Type: application/json' \
  -d '{"model":"grok-4.6","messages":[{"role":"user","content":"hi"}],"max_tokens":4,"stream":false}' \
  https://cli-chat-proxy.grok.com/v1/chat/completions \
  | grep -iE 'ratelimit|quota|remaining|reset|x-' | head -20
echo "(empty above = no rate-limit headers on chat either)"

echo
echo "=== 4. rate-limit endpoints used by grok clients ==="
for url in \
  "https://cli-chat-proxy.grok.com/v1/rate-limits" \
  "https://cli-chat-proxy.grok.com/rest/rate-limits" \
  "https://api.x.ai/v1/api-key" \
  "https://management-api.x.ai/auth/users/me" \
  "https://accounts.x.ai/api/quota" ; do
  code="$(curl -s -o /tmp/rl.out -w '%{http_code}' --max-time 15 "${H[@]}" "$url" || echo 000)"
  printf '%-52s %s  %s\n' "$url" "$code" "$(head -c 160 /tmp/rl.out | tr -d '\n')"
done

echo
echo "=== 5. POST rate-limits (grok.com web client shape) ==="
curl -s -o /tmp/rl2.out -w 'POST /rest/rate-limits = %{http_code}\n' --max-time 20 "${H[@]}" \
  -H 'Content-Type: application/json' \
  -d '{"requestKind":"DEFAULT","modelName":"grok-4.6"}' \
  https://cli-chat-proxy.grok.com/rest/rate-limits
head -c 300 /tmp/rl2.out; echo
