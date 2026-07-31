#!/usr/bin/env bash
set -euo pipefail
# Panda deploy — pull GHCR images and restart (no build on Panda).
TNEXUS_ROOT="${TNEXUS_ROOT:-/root/TNexus}"
COMPOSE_FILE="$TNEXUS_ROOT/deploy/panda/docker-compose.yml"
ENV_FILE="/opt/tnexus/.env"

if [[ ! -f "$ENV_FILE" ]]; then
  echo "missing $ENV_FILE" >&2
  exit 1
fi
if [[ ! -f "$COMPOSE_FILE" ]]; then
  echo "missing $COMPOSE_FILE — git pull $TNEXUS_ROOT first" >&2
  exit 1
fi

set -a
# shellcheck disable=SC1090
source "$ENV_FILE"
set +a

if [[ -x "$TNEXUS_ROOT/deploy/panda/export_pool.sh" ]]; then
  bash "$TNEXUS_ROOT/deploy/panda/export_pool.sh"
fi

docker compose --env-file "$ENV_FILE" -f "$COMPOSE_FILE" pull
docker compose --env-file "$ENV_FILE" -f "$COMPOSE_FILE" up -d --force-recreate api worker account-ops

sleep 4
curl -fsS http://127.0.0.1:9000/health
echo
curl -fsS http://127.0.0.1:9011/health 2>/dev/null || echo "account_ops not up"
echo
curl -fsS -o /dev/null -w "accounts_page=%{http_code}\n" https://tnexus.relai.asia/accounts || true
