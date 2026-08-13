#!/usr/bin/env bash
set -u
q() { docker exec -i new-api-postgres psql -U newapi -d new-api -A -F'|' -t -c "$1"; }

echo "--- abilities columns ---"
q "select column_name, data_type, is_nullable from information_schema.columns where table_name='abilities' order by ordinal_position;"

echo
echo "--- sample ability rows ---"
q "select * from abilities limit 3;"

echo
echo "--- how grok-4.6 is already priced/served ---"
q "select id, name, \"group\", models, base_url, priority, weight from channels where models like '%grok-4.6%';"
