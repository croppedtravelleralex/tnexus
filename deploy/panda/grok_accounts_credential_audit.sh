#!/usr/bin/env bash
# 审计 grok_web 账号的凭据完整性：缺 grok_credentials 行 / 缺 encrypted_primary 的账号无法提取 key。
set -euo pipefail

set -a; source /opt/tnexus/.env 2>/dev/null; set +a
DSN="${GROK_DATABASE_URL:-}"
[ -n "$DSN" ] || { echo "GROK_DATABASE_URL required" >&2; exit 1; }

echo "=== counts ==="
psql "$DSN" -At -c "
SELECT 'grok_web_total=' || count(*) FROM grok_accounts WHERE provider='grok_web';
"
psql "$DSN" -At -c "
SELECT 'missing_credential_row=' || count(*)
  FROM grok_accounts ga
  LEFT JOIN grok_credentials gc ON gc.account_id = ga.id
 WHERE ga.provider='grok_web' AND gc.account_id IS NULL;
"
psql "$DSN" -At -c "
SELECT 'empty_encrypted_primary=' || count(*)
  FROM grok_accounts ga
  JOIN grok_credentials gc ON gc.account_id = ga.id
 WHERE ga.provider='grok_web'
   AND (gc.encrypted_primary IS NULL OR length(gc.encrypted_primary) = 0);
"

echo "=== ids missing credential row ==="
psql "$DSN" -At -c "
SELECT string_agg(ga.id::text, ',' ORDER BY ga.id)
  FROM grok_accounts ga
  LEFT JOIN grok_credentials gc ON gc.account_id = ga.id
 WHERE ga.provider='grok_web' AND gc.account_id IS NULL;
"

echo "=== ids with empty encrypted_primary ==="
psql "$DSN" -At -c "
SELECT string_agg(ga.id::text, ',' ORDER BY ga.id)
  FROM grok_accounts ga
  JOIN grok_credentials gc ON gc.account_id = ga.id
 WHERE ga.provider='grok_web'
   AND (gc.encrypted_primary IS NULL OR length(gc.encrypted_primary) = 0);
"
