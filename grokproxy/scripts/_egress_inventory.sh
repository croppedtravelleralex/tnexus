#!/usr/bin/env bash
# What egress do stored accounts actually have, and does the relay answer?
set -u
db=/opt/grokproxy/data/grokproxy.db

echo "=== accounts by provider and whether they carry an egress ==="
sqlite3 -header -column "$db" "
  SELECT provider,
         CASE WHEN proxy_url='' THEN 'no proxy' ELSE 'has proxy' END AS egress,
         count(*) AS n
    FROM accounts GROUP BY 1,2;"

echo
echo "=== web accounts usable as a mint source (sso + proxy) ==="
sqlite3 -header -column "$db" "
  SELECT email, health, substr(proxy_url,1,50) AS proxy
    FROM accounts
   WHERE provider='web' AND sso_token<>'' AND proxy_url<>''
   LIMIT 5;"

echo
echo "=== relay configuration and whether it is listening ==="
grep -E 'STICKY_RELAY|GROKPROXY_PROXY' /opt/grokproxy/.env || echo "(none configured)"
ss -lntp 2>/dev/null | grep -E ':18100|:1810[0-9]' | head -5 || echo "(nothing on 18100)"
