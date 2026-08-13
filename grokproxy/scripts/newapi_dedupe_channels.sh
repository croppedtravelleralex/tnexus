#!/usr/bin/env bash
# Collapse duplicate grokproxy channels down to the lowest id.
#
# The first version of newapi_add_channel.sh used ON CONFLICT DO NOTHING on a
# table with no unique constraint on name, so each run added another row.
set -euo pipefail
NAME="${CHANNEL_NAME:-grokproxy}"

q() { docker exec -i new-api-postgres psql -U newapi -d new-api -A -t -c "$1"; }

keep="$(q "select min(id) from channels where name='${NAME}';" | tr -d '[:space:]')"
[[ -n "$keep" && "$keep" != "" ]] || { echo "no ${NAME} channel"; exit 0; }

dupes="$(q "select count(*) from channels where name='${NAME}' and id <> ${keep};" | tr -d '[:space:]')"
echo "keeping channel ${keep}, removing ${dupes} duplicate(s)"

docker exec -i new-api-postgres psql -U newapi -d new-api -v ON_ERROR_STOP=1 -q <<SQL
DELETE FROM abilities WHERE channel_id IN
  (SELECT id FROM channels WHERE name='${NAME}' AND id <> ${keep});
DELETE FROM channels WHERE name='${NAME}' AND id <> ${keep};
SQL

docker exec -i new-api-postgres psql -U newapi -d new-api -A -F'|' -t \
  -c "select id, name, \"group\", models, base_url from channels where name='${NAME}';"
