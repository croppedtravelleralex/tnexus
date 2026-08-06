#!/usr/bin/env bash
# Refresh gptimage account tokens and sync to TNexus Postgres (Panda ops).
# Safe to re-run; does not build anything.
set -euo pipefail

GPTIMAGE_CONTAINER="${GPTIMAGE_CONTAINER:-chatgpt2api-local}"
TNEXUS_ROOT="${TNEXUS_ROOT:-/root/TNexus}"
ACCOUNTS_DB="${ACCOUNTS_DB:-/root/gptimage/data/accounts.db}"
ENV_FILE="${ENV_FILE:-/opt/tnexus/.env}"

echo "==> refresh tokens in gptimage (sqlite)"
docker exec "$GPTIMAGE_CONTAINER" python3 - <<'PY'
import sys
sys.path.insert(0, "/app")
from services.account_service import account_service
from services.account_refresh_all_service import account_refresh_all_service

tokens = account_service.list_tokens()
print(f"accounts={len(tokens)}", flush=True)
if not tokens:
    raise SystemExit("no accounts in gptimage store")
result = account_service.refresh_accounts(tokens, None, False, False)
print(
    f"refreshed={result.get('refreshed', 0)} errors={len(result.get('errors') or [])}",
    flush=True,
)
sync = account_refresh_all_service.sync_last_refreshed_accounts_to_panda()
print(f"panda_sync={sync}", flush=True)
PY

if [[ -f "$TNEXUS_ROOT/scripts/etl_accounts_to_postgres.py" && -f "$ACCOUNTS_DB" ]]; then
  echo "==> ETL sqlite → postgres (full account data)"
  # shellcheck disable=SC1090
  set -a
  # shellcheck source=/dev/null
  source <(grep -E '^DATABASE_URL=' "$ENV_FILE" || true)
  set +a
  export ACCOUNTS_DB
  python3 "$TNEXUS_ROOT/scripts/etl_accounts_to_postgres.py"
fi

echo "==> done"
