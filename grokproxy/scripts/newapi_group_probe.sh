#!/usr/bin/env bash
# How does newapi decide a user may use a group?
set -u
q() { docker exec new-api-postgres psql -U newapi -d new-api -A -F'|' -t -c "$1"; }

echo "=== group-related options ==="
q "SELECT key, left(value, 600) FROM options
    WHERE key ILIKE '%group%' ORDER BY key;"

echo
echo "=== which groups do users actually hold ==="
q "SELECT \"group\", count(*) FROM users GROUP BY 1 ORDER BY 2 DESC LIMIT 10;"

echo
echo "=== the user behind our test tokens ==="
q "SELECT u.id, u.username, u.\"group\", u.status
     FROM users u JOIN tokens t ON t.user_id = u.id
    WHERE t.\"group\" = 'grok-claude' AND t.deleted_at IS NULL
    ORDER BY t.id DESC LIMIT 3;"

echo
echo "=== groups already used by channels (candidates that must be registered) ==="
q "SELECT DISTINCT trim(g) FROM channels, unnest(string_to_array(\"group\", ',')) AS g
   ORDER BY 1;"
