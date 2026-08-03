#!/usr/bin/env bash
set -euo pipefail
# Panda one-shot deploy — patch env, gateway, refresh JWT, pull GHCR, restart. No Python.
TNEXUS_ROOT="${TNEXUS_ROOT:-/root/TNexus}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
COMPOSE_FILE="$TNEXUS_ROOT/deploy/panda/docker-compose.yml"
ENV_FILE="/opt/tnexus/.env"

if [[ ! -f "$COMPOSE_FILE" ]]; then
  echo "missing $COMPOSE_FILE — git pull $TNEXUS_ROOT first" >&2
  exit 1
fi

if [[ ! -f "$ENV_FILE" ]]; then
  echo "bootstrapping $ENV_FILE from deploy/panda/.env.example" >&2
  mkdir -p /opt/tnexus/data/pool
  cp "$TNEXUS_ROOT/deploy/panda/.env.example" "$ENV_FILE"
  chmod 600 "$ENV_FILE"
fi

bash "$SCRIPT_DIR/patch_env.sh"
rm -f /opt/tnexus/data/pool/accounts_pool.json

# Gateway must be up before JWT refresh; then worker picks up new key on recreate.
bash "$SCRIPT_DIR/gateway-deploy.sh"
bash "$SCRIPT_DIR/refresh_upstream_jwt.sh"

set -a
# shellcheck disable=SC1090
source "$ENV_FILE"
set +a

docker compose --env-file "$ENV_FILE" -f "$COMPOSE_FILE" pull
docker compose --env-file "$ENV_FILE" -f "$COMPOSE_FILE" up -d --force-recreate api worker account-ops

# Postgres pool: api/worker no longer need gptimage sqlite when ACCOUNTS_BACKEND=postgres
if grep -q '^ACCOUNTS_BACKEND=postgres' "$ENV_FILE" 2>/dev/null; then
  echo "ACCOUNTS_BACKEND=postgres — ensure api compose dropped /gptimage/data mount in docker-compose.yml"
fi

sleep 4
curl -fsS http://127.0.0.1:9000/health
echo
curl -fsS http://127.0.0.1:9011/health 2>/dev/null || echo "account_ops not up"
echo
curl -fsS -o /dev/null -w "accounts_page=%{http_code}\n" https://tnexus.relai.asia/accounts || true
