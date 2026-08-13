#!/usr/bin/env bash
# Wire grok-4.6 into newapi twice: once as OpenAI, once as Anthropic.
#
# Same pool behind both — grokProxy serves /v1/chat/completions and
# /v1/messages from the same accounts — so a caller picks a protocol by
# choosing a group, not a different backend.
set -euo pipefail
cd /root/TNexus/grokproxy

set -a; . /opt/grokproxy/.env; set +a
export CHANNEL_KEY="${GROKPROXY_API_KEY:?grokProxy API key missing from /opt/grokproxy/.env}"
# newapi's own network gateway, so the route survives changes to the unrelated
# grok2api compose stack that 172.22.0.1 belongs to.
export CHANNEL_BASE_URL="${CHANNEL_BASE_URL:-http://172.19.0.1:8110}"

echo "=== OpenAI channel (type 1) ==="
CHANNEL_NAME=grokproxy \
CHANNEL_TYPE=1 \
CHANNEL_GROUP='default,grok' \
bash scripts/newapi_add_channel.sh

echo
echo "=== Anthropic channel (type 14) ==="
CHANNEL_NAME=grokproxy-anthropic \
CHANNEL_TYPE=14 \
CHANNEL_GROUP='grok-claude' \
bash scripts/newapi_add_channel.sh

echo
echo "=== all grok-4.6 routes now visible to the dispatcher ==="
docker exec -i new-api-postgres psql -U newapi -d new-api -A -F'|' -t -c "
  SELECT a.\"group\", a.model, c.id, c.name, c.type, c.base_url, a.enabled
    FROM abilities a JOIN channels c ON c.id = a.channel_id
   WHERE a.model = 'grok-4.6'
   ORDER BY c.id, a.\"group\";"
