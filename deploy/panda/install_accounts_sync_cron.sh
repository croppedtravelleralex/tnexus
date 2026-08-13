#!/usr/bin/env bash
# Install hourly gptimage->Postgres account pool sync (Panda only).
#
# Without this the gateway pool drifts: upstream refreshes access_tokens but Postgres
# keeps the old ones, and once they pass expiry every image request 401s.
set -euo pipefail

TNEXUS_ROOT="${TNEXUS_ROOT:-/root/TNexus}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CRON_LINE="23 * * * * root cd ${TNEXUS_ROOT} && bash ${SCRIPT_DIR}/sync_accounts_to_postgres.sh >>/var/log/tnexus-accounts-sync.log 2>&1"
CRON_FILE="/etc/cron.d/tnexus-accounts-sync"

if [[ "$(id -u)" -ne 0 ]]; then
  echo "run as root on Panda" >&2
  exit 1
fi

if [[ ! -f "${SCRIPT_DIR}/sync_accounts_to_postgres.sh" ]]; then
  echo "missing ${SCRIPT_DIR}/sync_accounts_to_postgres.sh" >&2
  exit 1
fi

printf '%s\n' "$CRON_LINE" >"$CRON_FILE"
chmod 644 "$CRON_FILE"
touch /var/log/tnexus-accounts-sync.log
chmod 644 /var/log/tnexus-accounts-sync.log

echo "installed $CRON_FILE"
echo "  $CRON_LINE"
echo "log: /var/log/tnexus-accounts-sync.log"
