#!/usr/bin/env bash
# Why does newapi reject a token that exists in its own database?
set -u
q() { docker exec new-api-postgres psql -U newapi -d new-api -A -F'|' -t -c "$1"; }

echo "=== tokens table columns ==="
docker exec new-api-postgres psql -U newapi -d new-api -c '\d tokens' | head -30

echo
echo "=== the grok-group tokens we might pick ==="
q "SELECT id, name, \"group\", status, unlimited_quota, remain_quota, expired_time,
           length(key), left(key,6), right(key,8), deleted_at
      FROM tokens WHERE \"group\" IN ('grok','grok-claude','default')
     ORDER BY \"group\", id DESC LIMIT 12;"

echo
echo "=== does the key column already include a prefix? ==="
q "SELECT DISTINCT left(key,3) AS prefix, count(*) FROM tokens GROUP BY 1 ORDER BY 2 DESC LIMIT 5;"

echo
echo "=== the row the e2e script created ==="
q "SELECT id, user_id, name, \"group\", status, unlimited_quota, expired_time,
           length(key), deleted_at
      FROM tokens WHERE name LIKE 'grokproxy-anthropic%' ORDER BY id DESC LIMIT 3;"

echo
echo "=== newapi's own view: is the user behind these tokens enabled? ==="
q "SELECT t.id, t.name, t.user_id, u.username, u.status AS user_status, u.\"group\" AS user_group
     FROM tokens t LEFT JOIN users u ON u.id = t.user_id
    WHERE t.\"group\" IN ('grok','grok-claude') ORDER BY t.id DESC LIMIT 8;"

echo
echo "=== what newapi logs when the call is rejected ==="
docker logs --since 10m new-api 2>&1 | grep -iE 'invalid token|token.*not|unauthor' | tail -5
