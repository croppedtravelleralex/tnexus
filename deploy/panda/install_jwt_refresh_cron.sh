#!/usr/bin/env bash
# Install daily gateway JWT refresh + NewAPI channel key sync (Panda only).
set -euo pipefail

TNEXUS_ROOT="${TNEXUS_ROOT:-/root/TNexus}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CRON_LINE="17 4 * * * root cd ${TNEXUS_ROOT} && bash ${SCRIPT_DIR}/refresh_upstream_jwt.sh >>/var/log/tnexus-jwt-refresh.log 2>&1"
CRON_FILE="/etc/cron.d/tnexus-jwt-refresh"

if [[ "$(id -u)" -ne 0 ]]; then
  echo "run as root on Panda" >&2
  exit 1
fi

if [[ ! -x "${SCRIPT_DIR}/refresh_upstream_jwt.sh" ]]; then
  echo "missing ${SCRIPT_DIR}/refresh_upstream_jwt.sh" >&2
  exit 1
fi

printf '%s\n' "$CRON_LINE" >"$CRON_FILE"
chmod 644 "$CRON_FILE"
touch /var/log/tnexus-jwt-refresh.log
chmod 644 /var/log/tnexus-jwt-refresh.log

echo "installed $CRON_FILE"
echo "  $CRON_LINE"
echo "log: /var/log/tnexus-jwt-refresh.log"
