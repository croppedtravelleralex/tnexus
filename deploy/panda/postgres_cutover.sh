#!/usr/bin/env bash
# One-shot: migration 009 → ETL sqlite → reconcile → switch ACCOUNTS_BACKEND=postgres
set -euo pipefail

TNEXUS_ROOT="${TNEXUS_ROOT:-/root/TNexus}"
ENV_FILE="/opt/tnexus/.env"
SQLITE_PATH="${SQLITE_PATH:-/root/gptimage/data/accounts.db}"
MIGRATION="$TNEXUS_ROOT/migrations/009_tnexus_accounts.sql"

if [[ ! -f "$ENV_FILE" ]]; then
  echo "missing $ENV_FILE" >&2
  exit 1
fi
if [[ ! -f "$MIGRATION" ]]; then
  echo "missing $MIGRATION — sync or git pull $TNEXUS_ROOT" >&2
  exit 1
fi

set -a
# shellcheck disable=SC1090
source "$ENV_FILE"
set +a

if [[ -z "${DATABASE_URL:-}" ]]; then
  echo "DATABASE_URL required in $ENV_FILE" >&2
  exit 1
fi

PG_CONTAINER="${PG_CONTAINER:-panda-postgres-1}"

python3 -c "import psycopg2" 2>/dev/null || pip3 install -q psycopg2-binary

echo "==> apply migration 009"
docker exec -i "$PG_CONTAINER" psql -U tnexus -d tnexus <"$MIGRATION"

if [[ ! -f "$SQLITE_PATH" ]]; then
  echo "sqlite not found: $SQLITE_PATH" >&2
  exit 1
fi

echo "==> ETL sqlite → postgres"
export ACCOUNTS_DB="$SQLITE_PATH"
python3 "$TNEXUS_ROOT/scripts/etl_accounts_to_postgres.py"

echo "==> reconcile counts"
export ACCOUNTS_DB="$SQLITE_PATH"
python3 "$TNEXUS_ROOT/scripts/reconcile_accounts_postgres.py" || true

echo "==> patch $ENV_FILE"
if grep -q '^ACCOUNTS_BACKEND=' "$ENV_FILE"; then
  sed -i 's/^ACCOUNTS_BACKEND=.*/ACCOUNTS_BACKEND=postgres/' "$ENV_FILE"
else
  echo "ACCOUNTS_BACKEND=postgres" >>"$ENV_FILE"
fi
if grep -q '^ACCOUNTS_DB=' "$ENV_FILE"; then
  sed -i "s|^ACCOUNTS_DB=.*|ACCOUNTS_DB=${DATABASE_URL}|" "$ENV_FILE"
else
  echo "ACCOUNTS_DB=${DATABASE_URL}" >>"$ENV_FILE"
fi

echo "done. ACCOUNTS_* now:"
grep -E '^ACCOUNTS_' "$ENV_FILE"

echo "==> redeploy TNexus (gateway + api/worker pick up postgres env)"
bash "$TNEXUS_ROOT/deploy/panda/deploy.sh"
