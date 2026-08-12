#!/usr/bin/env bash
set -euo pipefail
# Pull TNexus gateway image (scheduling_gate) and restart :8014.
TNEXUS_ROOT="${TNEXUS_ROOT:-/root/TNexus}"
COMPOSE_FILE="$TNEXUS_ROOT/deploy/panda/gateway-compose.yml"
ENV_FILE="/opt/tnexus/.env"

if [[ ! -f "$ENV_FILE" ]]; then
  echo "missing $ENV_FILE" >&2
  exit 1
fi

set -a
# shellcheck disable=SC1090
source "$ENV_FILE"
set +a

export TNEXUS_GATEWAY_IMAGE="${TNEXUS_GATEWAY_IMAGE:-ghcr.io/croppedtravelleralex/tnexus-gateway:latest}"

# Retire legacy gptimage-gateway-rs compose container if present.
docker stop deploy-gateway-1 2>/dev/null || true
docker rm deploy-gateway-1 2>/dev/null || true

docker compose --env-file "$ENV_FILE" -f "$COMPOSE_FILE" pull
docker compose --env-file "$ENV_FILE" -f "$COMPOSE_FILE" up -d --force-recreate

sleep 2
curl -fsS http://127.0.0.1:8014/health
echo

# Gateway recreate wipes ephemeral /data/auth.db; refresh worker JWT to match bootstrap user.
# Test with -f, not -x: git stores these as 0644, so an -x guard skips the refresh
# and leaves NewAPI holding a JWT the freshly recreated gateway rejects.
if [[ -f "$TNEXUS_ROOT/deploy/panda/refresh_upstream_jwt.sh" ]]; then
  bash "$TNEXUS_ROOT/deploy/panda/refresh_upstream_jwt.sh"
fi
if [[ -f "$TNEXUS_ROOT/deploy/panda/docker-compose.yml" ]]; then
  docker compose --env-file "$ENV_FILE" -f "$TNEXUS_ROOT/deploy/panda/docker-compose.yml" up -d worker api
fi
