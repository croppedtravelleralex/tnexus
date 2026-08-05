#!/usr/bin/env bash
# NewAPI Phase-2 gray: split gpt-image traffic between :8012 and TNexus gateway :8014.
# Run on Panda after gateway :8014 is healthy. Rollback: bash newapi_grayd_tnexus.sh rollback
set -euo pipefail

ENV_FILE="${ENV_FILE:-/opt/tnexus/.env}"
PG_CONTAINER="${PG_CONTAINER:-new-api-postgres}"
DB_USER="${NEWAPI_DB_USER:-newapi}"
DB_NAME="${NEWAPI_DB_NAME:-new-api}"
CHANNEL_NAME="${TNEXUS_CHANNEL_NAME:-tnexus-gateway}"
LEGACY_CHANNEL_ID="${LEGACY_CHANNEL_ID:-84}"
BASE_URL="${TNEXUS_GATEWAY_BASE:-http://host.docker.internal:8014}"
GRAY_WEIGHT="${TNEXUS_GRAY_WEIGHT:-30}"
LEGACY_WEIGHT="${LEGACY_GRAY_WEIGHT:-70}"

sql_escape() {
  printf '%s' "$1" | sed "s/'/''/g"
}

psql_exec() {
  docker exec "$PG_CONTAINER" psql -U "$DB_USER" -d "$DB_NAME" -v ON_ERROR_STOP=1 -c "$1"
}

load_gateway_key() {
  if [[ ! -f "$ENV_FILE" ]]; then
    echo "missing $ENV_FILE" >&2
    exit 1
  fi
  GATEWAY_KEY=$(grep '^GATEWAY_AUTH_KEY=' "$ENV_FILE" | cut -d= -f2- || true)
  if [[ -z "$GATEWAY_KEY" ]]; then
    echo "GATEWAY_AUTH_KEY missing in $ENV_FILE — run deploy.sh first" >&2
    exit 1
  fi
}

sync_tnexus_channel_key() {
  local key_esc base_esc
  key_esc=$(sql_escape "$GATEWAY_KEY")
  base_esc=$(sql_escape "$BASE_URL")
  psql_exec "UPDATE channels SET key = '${key_esc}', base_url = '${base_esc}', status = 1 WHERE name = '${CHANNEL_NAME}';"
}

rollback() {
  echo "==> rollback: remove $CHANNEL_NAME, restore channel $LEGACY_CHANNEL_ID weight=100"
  psql_exec "DELETE FROM channels WHERE name = '${CHANNEL_NAME}';"
  psql_exec "UPDATE channels SET weight = 100 WHERE id = ${LEGACY_CHANNEL_ID};"
  echo "rollback done"
}

apply_gray() {
  load_gateway_key
  if ! curl -fsS -o /dev/null --max-time 5 http://127.0.0.1:8014/health; then
    echo "gateway :8014 not healthy — abort" >&2
    exit 1
  fi

  LEGACY_ROW=$(docker exec "$PG_CONTAINER" psql -U "$DB_USER" -d "$DB_NAME" -t -A -F '|' \
    -c "SELECT models, \"group\", priority, type, status, auto_ban FROM channels WHERE id = ${LEGACY_CHANNEL_ID};")
  if [[ -z "$LEGACY_ROW" ]]; then
    echo "legacy channel $LEGACY_CHANNEL_ID not found" >&2
    exit 1
  fi
  IFS='|' read -r MODELS CHANNEL_GROUP PRIORITY CH_TYPE CH_STATUS AUTO_BAN <<<"$LEGACY_ROW"
  CREATED=$(date +%s)
  KEY_ESC=$(sql_escape "$GATEWAY_KEY")
  BASE_ESC=$(sql_escape "$BASE_URL")
  GROUP_ESC=$(sql_escape "$CHANNEL_GROUP")
  MODELS_ESC=$(sql_escape "$MODELS")

  EXISTING_ID=$(docker exec "$PG_CONTAINER" psql -U "$DB_USER" -d "$DB_NAME" -t -A \
    -c "SELECT id FROM channels WHERE name = '${CHANNEL_NAME}' LIMIT 1;" | tr -d '[:space:]')

  if [[ -n "$EXISTING_ID" ]]; then
    echo "==> update channel id=$EXISTING_ID ($CHANNEL_NAME)"
    psql_exec "UPDATE channels SET key = '${KEY_ESC}', base_url = '${BASE_ESC}', weight = ${GRAY_WEIGHT}, status = 1 WHERE id = ${EXISTING_ID};"
  else
    echo "==> insert channel $CHANNEL_NAME → $BASE_URL (weight $GRAY_WEIGHT)"
    psql_exec "INSERT INTO channels (type, key, status, name, weight, created_time, base_url, models, \"group\", priority, auto_ban) VALUES (${CH_TYPE}, '${KEY_ESC}', ${CH_STATUS}, '${CHANNEL_NAME}', ${GRAY_WEIGHT}, ${CREATED}, '${BASE_ESC}', '${MODELS_ESC}', '${GROUP_ESC}', ${PRIORITY}, ${AUTO_BAN});"
  fi

  psql_exec "UPDATE channels SET weight = ${LEGACY_WEIGHT} WHERE id = ${LEGACY_CHANNEL_ID};"

  echo "==> channels (8012 vs :8014)"
  docker exec "$PG_CONTAINER" psql -U "$DB_USER" -d "$DB_NAME" \
    -c "SELECT id, name, base_url, weight, status FROM channels WHERE id = ${LEGACY_CHANNEL_ID} OR name = '${CHANNEL_NAME}' ORDER BY id;"
  echo "gray apply done (${GRAY_WEIGHT}% TNexus / ${LEGACY_WEIGHT}% :8012 by weight)"
}

case "${1:-apply}" in
  rollback) rollback ;;
  sync-key)
    load_gateway_key
    sync_tnexus_channel_key
    echo "synced $CHANNEL_NAME key from GATEWAY_AUTH_KEY"
    ;;
  apply|"") apply_gray ;;
  *) echo "usage: $0 [apply|rollback|sync-key]" >&2; exit 1 ;;
esac
