#!/usr/bin/env bash
# End-to-end: newapi -> grokproxy -> account pool -> upstream.
#
# Uses an existing newapi token that already has access to the target group, so
# the test exercises real routing rather than a hand-made shortcut.
set -u
MODEL="${TEST_MODEL:-grok-4.6}"
GROUP="${TEST_GROUP:-grok}"

q() { docker exec -i new-api-postgres psql -U newapi -d new-api -A -t -c "$1"; }

echo "--- channel + ability ---"
docker exec -i new-api-postgres psql -U newapi -d new-api -A -F'|' -t \
  -c "select c.id, c.name, c.status, c.\"group\", c.models, c.base_url from channels c where c.models like '%${MODEL}%';"

echo
echo "--- a token whose group can see ${GROUP} ---"
token="$(q "select key from tokens where status=1 and (\"group\"='${GROUP}' or \"group\"='' or \"group\" is null) order by id desc limit 1;" | tr -d '[:space:]')"
if [[ -z "$token" ]]; then
  echo "no usable token found; create one in the newapi UI with group=${GROUP}" >&2
  exit 1
fi
echo "token found (${#token} chars)"

echo
echo "--- calling newapi ---"
start=$(date +%s)
body=$(curl -s --max-time 300 http://127.0.0.1:8081/v1/chat/completions \
  -H "Authorization: Bearer sk-${token}" \
  -H 'Content-Type: application/json' \
  -d "{\"model\":\"${MODEL}\",\"messages\":[{\"role\":\"user\",\"content\":\"Reply with exactly NEWAPI_OK\"}],\"max_tokens\":8,\"stream\":false}")
elapsed=$(( $(date +%s) - start ))
echo "elapsed=${elapsed}s"
printf '%s\n' "$body" | head -c 600
echo
