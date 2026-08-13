#!/usr/bin/env bash
# Read-only look at newapi's channels/groups so a new channel matches the
# conventions already in use (naming, group names, pricing style).
set -u
PSQL=(docker exec new-api-postgres psql -U newapi -d new-api -A -F'|' -t -c)

echo "--- channel columns ---"
"${PSQL[@]}" "select column_name, data_type from information_schema.columns where table_name='channels' order by ordinal_position;" \
  | head -40

echo
echo "--- existing channels (no keys) ---"
"${PSQL[@]}" "select id, name, type, status, \"group\", coalesce(base_url,'') from channels order by id desc limit 15;"

echo
echo "--- distinct groups ---"
"${PSQL[@]}" "select distinct \"group\" from channels;"

echo
echo "--- any grok models already? ---"
"${PSQL[@]}" "select id, name, left(models, 120) from channels where models ilike '%grok%' limit 10;"

echo
echo "--- pricing table present? ---"
"${PSQL[@]}" "select table_name from information_schema.tables where table_name in ('prices','model_prices','pricings','abilities');"
