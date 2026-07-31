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
