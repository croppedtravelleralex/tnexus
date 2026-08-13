#!/usr/bin/env bash
# Sync gptimage account pool (sqlite, source of truth) -> Postgres tnexus_accounts.
#
# The gateway reads its image pool from Postgres (ACCOUNTS_BACKEND=postgres). Upstream
# gptimage refreshes access_tokens on its own schedule, but nothing propagated those
# refreshed tokens to Postgres, so the gateway kept serving expired tokens and every
# image request failed with `chat_requirements_prepare HTTP 401`.
#
# Panda-only. Pull/sync only, never builds.
set -euo pipefail

TNEXUS_ROOT="${TNEXUS_ROOT:-/root/TNexus}"
ENV_FILE="${ENV_FILE:-/opt/tnexus/.env}"
SQLITE_DB="${GPTIMAGE_ACCOUNTS_DB:-/root/gptimage/data/accounts.db}"

if [[ ! -f "$SQLITE_DB" ]]; then
  echo "[$(date -Is)] FATAL: sqlite pool not found: $SQLITE_DB" >&2
  exit 1
fi

# NOTE: read DATABASE_URL out of the env file rather than sourcing it. The env file also
# defines ACCOUNTS_DB as a *Postgres* URL, but etl_accounts_to_postgres.py expects
# ACCOUNTS_DB to be the *sqlite* path — sourcing would silently break the ETL.
PG_URL="$(grep -E '^DATABASE_URL=' "$ENV_FILE" | head -1 | cut -d= -f2- | tr -d '\r\n')"
if [[ -z "$PG_URL" ]]; then
  echo "[$(date -Is)] FATAL: DATABASE_URL missing from $ENV_FILE" >&2
  exit 1
fi

cd "$TNEXUS_ROOT"
echo "[$(date -Is)] syncing $SQLITE_DB -> tnexus_accounts"
ACCOUNTS_DB="$SQLITE_DB" DATABASE_URL="$PG_URL" python3 scripts/etl_accounts_to_postgres.py

# Report how many pooled tokens are still expired so the log shows drift immediately.
ACCOUNTS_DB="$SQLITE_DB" DATABASE_URL="$PG_URL" python3 - <<'PY'
import base64, json, os, time
import psycopg2

def jwt_exp(tok):
    try:
        p = tok.split(".")[1]; p += "=" * (-len(p) % 4)
        return json.loads(base64.urlsafe_b64decode(p)).get("exp")
    except Exception:
        return None

conn = psycopg2.connect(os.environ["DATABASE_URL"])
with conn.cursor() as cur:
    cur.execute("SELECT email, access_token, data->>'status' FROM tnexus_accounts")
    rows = cur.fetchall()
conn.close()

now = int(time.time())
expired = [e for e, t, _ in rows if (lambda x: x is None or x < now)(jwt_exp(t))]
blocked = [e for e, _, s in rows if (s or "") != "正常"]
print(f"pool={len(rows)} expired={len(expired)} non_schedulable={len(blocked)}")
if expired:
    print("  expired: " + ", ".join(sorted(expired)[:10]))
PY

echo "[$(date -Is)] sync done"
