#!/usr/bin/env bash
# Register grokProxy as an OpenAI-compatible channel in newapi.
#
# Idempotent: re-running updates the existing row instead of adding a duplicate.
# The abilities table is what actually routes model -> channel, so it is
# rebuilt alongside the channel row.
set -euo pipefail

NAME="${CHANNEL_NAME:-grokproxy}"
GROUP="${CHANNEL_GROUP:-grok}"
MODELS="${CHANNEL_MODELS:-grok-4.6}"
BASE_URL="${CHANNEL_BASE_URL:-http://172.22.0.1:8110}"
KEY="${CHANNEL_KEY:?CHANNEL_KEY (grokProxy API key) required}"
PRIORITY="${CHANNEL_PRIORITY:-0}"
WEIGHT="${CHANNEL_WEIGHT:-1}"

psql_do() {
  docker exec -i new-api-postgres psql -U newapi -d new-api -v ON_ERROR_STOP=1 "$@"
}

now="$(date +%s)"

# `channels` has no unique constraint on name, so ON CONFLICT cannot dedupe:
# insert only when the name is absent, then update. Without this, re-running
# the script silently creates duplicate channels that all serve the same pool.
psql_do -q <<SQL
INSERT INTO channels
  (type, key, name, status, base_url, models, "group", priority, weight,
   created_time, auto_ban, model_mapping, other, test_model)
SELECT 1, '${KEY}', '${NAME}', 1, '${BASE_URL}', '${MODELS}', '${GROUP}',
       ${PRIORITY}, ${WEIGHT}, ${now}, 1, '', '', 'grok-4.6'
 WHERE NOT EXISTS (SELECT 1 FROM channels WHERE name = '${NAME}');

UPDATE channels
   SET key = '${KEY}',
       base_url = '${BASE_URL}',
       models = '${MODELS}',
       "group" = '${GROUP}',
       status = 1,
       priority = ${PRIORITY},
       weight = ${WEIGHT},
       test_model = 'grok-4.6'
 WHERE name = '${NAME}';
SQL

channel_id="$(docker exec -i new-api-postgres psql -U newapi -d new-api -A -t \
  -c "select id from channels where name='${NAME}' limit 1;" | tr -d '[:space:]')"
[[ -n "$channel_id" ]] || { echo "channel not found after upsert" >&2; exit 1; }

# abilities is newapi's routing index; a channel with no ability row is invisible
# to the dispatcher even though it looks configured in the UI.
psql_do -q <<SQL
DELETE FROM abilities WHERE channel_id = ${channel_id};
INSERT INTO abilities ("group", model, channel_id, enabled, priority, weight)
SELECT trim(g), trim(m), ${channel_id}, true, ${PRIORITY}, ${WEIGHT}
  FROM unnest(string_to_array('${GROUP}', ',')) AS g,
       unnest(string_to_array('${MODELS}', ',')) AS m;
SQL

echo "channel ${channel_id} (${NAME}) -> ${BASE_URL} models=${MODELS} group=${GROUP}"
docker exec -i new-api-postgres psql -U newapi -d new-api -A -F'|' -t \
  -c "select id, name, type, status, \"group\", models, base_url from channels where id=${channel_id};"
docker exec -i new-api-postgres psql -U newapi -d new-api -A -F'|' -t \
  -c "select \"group\", model, channel_id, enabled from abilities where channel_id=${channel_id};"
