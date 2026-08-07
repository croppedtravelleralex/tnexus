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

all_tokens = account_service.list_tokens()
eligible = []
skipped = 0
for token in all_tokens:
    account = account_service.get_account(token) or {}
    if str(account.get("last_token_refresh_error") or "").strip():
        skipped += 1
        continue
    eligible.append(token)
print(f"accounts={len(all_tokens)} eligible={len(eligible)} skipped_refresh_error={skipped}", flush=True)
if not eligible:
    raise SystemExit("no eligible accounts to refresh (all have last_token_refresh_error)")
result = account_service.refresh_accounts(eligible, None, False, False)
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
