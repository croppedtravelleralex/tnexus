#!/usr/bin/env bash
# NewAPI dedicated TNexus group: token group "tnexus" → 100% gateway :8014 (no gray with 生图).
# Does NOT touch gray channels 84/114 — production 生图灰度 stays as-is.
# Run on Panda after gateway :8014 is healthy.
#
#   bash newapi_tnexus_dedicated.sh apply     # create group + channel + token
#   bash newapi_tnexus_dedicated.sh sync-key    # refresh channel key from GATEWAY_AUTH_KEY
#   bash newapi_tnexus_dedicated.sh status      # show channel + token
#   bash newapi_tnexus_dedicated.sh rollback    # remove dedicated group setup
set -euo pipefail

ENV_FILE="${ENV_FILE:-/opt/tnexus/.env}"
PG_CONTAINER="${PG_CONTAINER:-new-api-postgres}"
DB_USER="${NEWAPI_DB_USER:-newapi}"
DB_NAME="${NEWAPI_DB_NAME:-new-api}"
TNEXUS_GROUP="${TNEXUS_GROUP:-tnexus}"
CHANNEL_NAME="${TNEXUS_DEDICATED_CHANNEL:-tnexus-dedicated}"
TOKEN_NAME="${TNEXUS_DEDICATED_TOKEN:-tnexus-test-key}"
BASE_URL="${TNEXUS_GATEWAY_BASE:-http://host.docker.internal:8014}"
MODELS="${TNEXUS_DEDICATED_MODELS:-gpt-image-2}"
GROUP_RATIO="${TNEXUS_GROUP_RATIO:-0.1}"
TOKEN_USER_ID="${TNEXUS_TOKEN_USER_ID:-1}"

sql_escape() {
  printf '%s' "$1" | sed "s/'/''/g"
}

psql_exec() {
  docker exec "$PG_CONTAINER" psql -U "$DB_USER" -d "$DB_NAME" -v ON_ERROR_STOP=1 -c "$1"
}

psql_scalar() {
  docker exec "$PG_CONTAINER" psql -U "$DB_USER" -d "$DB_NAME" -t -A -v ON_ERROR_STOP=1 -c "$1" | tr -d '[:space:]'
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

merge_option_json() {
  local key="$1" json_key="$2" json_val="$3"
  docker exec -i "$PG_CONTAINER" psql -U "$DB_USER" -d "$DB_NAME" -v ON_ERROR_STOP=1 <<SQL
DO \$\$
DECLARE
  raw text;
  obj jsonb;
BEGIN
  SELECT value INTO raw FROM options WHERE key = '${key}' LIMIT 1;
  IF raw IS NULL THEN
    obj := '{}'::jsonb;
  ELSE
    obj := raw::jsonb;
  END IF;
  obj := obj || jsonb_build_object('${json_key}', '${json_val}');
  IF EXISTS (SELECT 1 FROM options WHERE key = '${key}') THEN
    UPDATE options SET value = obj::text WHERE key = '${key}';
  ELSE
    INSERT INTO options (key, value) VALUES ('${key}', obj::text);
  END IF;
END \$\$;
SQL
}

merge_option_json_number() {
  local key="$1" json_key="$2" json_val="$3"
  docker exec -i "$PG_CONTAINER" psql -U "$DB_USER" -d "$DB_NAME" -v ON_ERROR_STOP=1 <<SQL
DO \$\$
DECLARE
  raw text;
  obj jsonb;
BEGIN
  SELECT value INTO raw FROM options WHERE key = '${key}' LIMIT 1;
  IF raw IS NULL THEN
    obj := '{}'::jsonb;
  ELSE
    obj := raw::jsonb;
  END IF;
  obj := obj || jsonb_build_object('${json_key}', ${json_val}::numeric);
  IF EXISTS (SELECT 1 FROM options WHERE key = '${key}') THEN
    UPDATE options SET value = obj::text WHERE key = '${key}';
  ELSE
    INSERT INTO options (key, value) VALUES ('${key}', obj::text);
  END IF;
END \$\$;
SQL
}

remove_option_json_key() {
  local key="$1" json_key="$2"
  docker exec -i "$PG_CONTAINER" psql -U "$DB_USER" -d "$DB_NAME" -v ON_ERROR_STOP=1 <<SQL
DO \$\$
DECLARE
  raw text;
  obj jsonb;
BEGIN
  SELECT value INTO raw FROM options WHERE key = '${key}' LIMIT 1;
  IF raw IS NULL THEN
    RETURN;
  END IF;
  obj := raw::jsonb - '${json_key}';
  UPDATE options SET value = obj::text WHERE key = '${key}';
END \$\$;
SQL
}

gen_token_key() {
  openssl rand -base64 48 | tr -d '/+=' | head -c 48
}

sync_channel_key() {
  local key_esc base_esc
  load_gateway_key
  key_esc=$(sql_escape "$GATEWAY_KEY")
  base_esc=$(sql_escape "$BASE_URL")
  psql_exec "UPDATE channels SET key = '${key_esc}', base_url = '${base_esc}', status = 1 WHERE name = '${CHANNEL_NAME}';"
}

upsert_channel() {
  local key_esc base_esc models_esc created existing_id
  load_gateway_key
  key_esc=$(sql_escape "$GATEWAY_KEY")
  base_esc=$(sql_escape "$BASE_URL")
  models_esc=$(sql_escape "$MODELS")
  created=$(date +%s)
  existing_id=$(psql_scalar "SELECT id FROM channels WHERE name = '${CHANNEL_NAME}' LIMIT 1;")

  if [[ -n "$existing_id" ]]; then
    echo "==> update channel id=$existing_id ($CHANNEL_NAME)"
    psql_exec "UPDATE channels SET key = '${key_esc}', base_url = '${base_esc}', models = '${models_esc}', \"group\" = '${TNEXUS_GROUP}', weight = 100, status = 1 WHERE id = ${existing_id};"
  else
    echo "==> insert channel $CHANNEL_NAME → $BASE_URL (group=${TNEXUS_GROUP}, weight=100)"
    psql_exec "INSERT INTO channels (type, key, status, name, weight, created_time, base_url, models, \"group\", priority, auto_ban) VALUES (1, '${key_esc}', 1, '${CHANNEL_NAME}', 100, ${created}, '${base_esc}', '${models_esc}', '${TNEXUS_GROUP}', 100, 0);"
    existing_id=$(psql_scalar "SELECT id FROM channels WHERE name = '${CHANNEL_NAME}' LIMIT 1;")
  fi
  CHANNEL_ID="$existing_id"
}

upsert_abilities() {
  local model
  IFS=',' read -ra MODEL_ARR <<<"$MODELS"
  for model in "${MODEL_ARR[@]}"; do
    model=$(echo "$model" | xargs)
    [[ -z "$model" ]] && continue
    psql_exec "INSERT INTO abilities (\"group\", model, channel_id, enabled, priority, weight)
      VALUES ('${TNEXUS_GROUP}', '${model}', ${CHANNEL_ID}, true, 100, 100)
      ON CONFLICT (\"group\", model, channel_id) DO UPDATE
      SET enabled = EXCLUDED.enabled, priority = EXCLUDED.priority, weight = EXCLUDED.weight;"
  done
}

upsert_token() {
  local token_key created name_esc key_esc existing_key
  created=$(date +%s)
  name_esc=$(sql_escape "$TOKEN_NAME")
  existing_key=$(psql_scalar "SELECT key FROM tokens WHERE name = '${name_esc}' AND deleted_at IS NULL LIMIT 1;")

  if [[ -n "$existing_key" ]]; then
    echo "==> token already exists: $TOKEN_NAME"
    TOKEN_KEY="$existing_key"
    psql_exec "UPDATE tokens SET \"group\" = '${TNEXUS_GROUP}', status = 1, unlimited_quota = true WHERE name = '${name_esc}' AND deleted_at IS NULL;"
    return
  fi

  token_key=$(gen_token_key)
  key_esc=$(sql_escape "$token_key")
  echo "==> create token $TOKEN_NAME (group=${TNEXUS_GROUP})"
  psql_exec "INSERT INTO tokens (user_id, key, status, name, created_time, remain_quota, unlimited_quota, \"group\")
    VALUES (${TOKEN_USER_ID}, '${key_esc}', 1, '${name_esc}', ${created}, 0, true, '${TNEXUS_GROUP}');"
  TOKEN_KEY="$token_key"
}

apply_dedicated() {
  if ! curl -fsS -o /dev/null --max-time 5 http://127.0.0.1:8014/health; then
    echo "gateway :8014 not healthy — abort" >&2
    exit 1
  fi

  echo "==> register group ${TNEXUS_GROUP} in NewAPI options"
  merge_option_json "UserUsableGroups" "$TNEXUS_GROUP" "$MODELS"
  merge_option_json_number "GroupRatio" "$TNEXUS_GROUP" "$GROUP_RATIO"

  upsert_channel
  upsert_abilities
  upsert_token

  if docker ps --format '{{.Names}}' | grep -qx 'new-api'; then
    echo "==> restart new-api (reload UserUsableGroups)"
    docker restart new-api >/dev/null
    for _ in $(seq 1 20); do
      if curl -fsS -o /dev/null --max-time 2 http://127.0.0.1:8081/api/status 2>/dev/null; then
        break
      fi
      sleep 2
    done
  fi

  echo ""
  echo "==> dedicated TNexus setup complete"
  echo "    group:   ${TNEXUS_GROUP}"
  echo "    channel: ${CHANNEL_NAME} (id=${CHANNEL_ID}) → ${BASE_URL}"
  echo "    token:   ${TOKEN_NAME}"
  echo "    key:     ${TOKEN_KEY}"
  echo ""
  echo "Test (NewAPI loopback :8081; sub2api must whitelist group ${TNEXUS_GROUP} separately):"
  echo "  curl -sS http://127.0.0.1:8081/v1/images/generations \\"
  echo "    -H 'Authorization: Bearer ${TOKEN_KEY}' \\"
  echo "    -H 'Content-Type: application/json' \\"
  echo "    -d '{\"model\":\"gpt-image-2\",\"prompt\":\"a red apple\",\"n\":1,\"size\":\"1024x1024\"}'"
  echo ""
  status_dedicated
}

rollback_dedicated() {
  echo "==> rollback dedicated TNexus group"
  remove_option_json_key "UserUsableGroups" "$TNEXUS_GROUP"
  remove_option_json_key "GroupRatio" "$TNEXUS_GROUP"

  local ch_id
  ch_id=$(psql_scalar "SELECT id FROM channels WHERE name = '${CHANNEL_NAME}' LIMIT 1;" || true)
  if [[ -n "$ch_id" ]]; then
    psql_exec "DELETE FROM abilities WHERE channel_id = ${ch_id};"
    psql_exec "DELETE FROM channels WHERE id = ${ch_id};"
  fi

  psql_exec "UPDATE tokens SET status = 2 WHERE name = '$(sql_escape "$TOKEN_NAME")' AND deleted_at IS NULL;"
  echo "rollback done (token disabled, channel removed; 生图 gray unchanged)"
}

status_dedicated() {
  echo "==> options"
  docker exec "$PG_CONTAINER" psql -U "$DB_USER" -d "$DB_NAME" \
    -c "SELECT key, value FROM options WHERE key IN ('UserUsableGroups','GroupRatio');"
  echo "==> channel"
  docker exec "$PG_CONTAINER" psql -U "$DB_USER" -d "$DB_NAME" \
    -c "SELECT id, name, \"group\", weight, base_url, status FROM channels WHERE name = '${CHANNEL_NAME}';"
  echo "==> abilities"
  docker exec "$PG_CONTAINER" psql -U "$DB_USER" -d "$DB_NAME" \
    -c "SELECT \"group\", model, channel_id, enabled, weight FROM abilities WHERE \"group\" = '${TNEXUS_GROUP}';"
  echo "==> token"
  docker exec "$PG_CONTAINER" psql -U "$DB_USER" -d "$DB_NAME" \
    -c "SELECT id, name, \"group\", status, unlimited_quota, key FROM tokens WHERE name = '$(sql_escape "$TOKEN_NAME")' AND deleted_at IS NULL;"
  echo "==> gray channels (unchanged by this script)"
  docker exec "$PG_CONTAINER" psql -U "$DB_USER" -d "$DB_NAME" \
    -c "SELECT id, name, \"group\", weight, base_url FROM channels WHERE id IN (84, 114) ORDER BY id;"
}

case "${1:-apply}" in
  apply|"") apply_dedicated ;;
  sync-key)
    sync_channel_key
    echo "synced ${CHANNEL_NAME} key from GATEWAY_AUTH_KEY"
    ;;
  status) status_dedicated ;;
  rollback) rollback_dedicated ;;
  *)
    echo "usage: $0 [apply|sync-key|status|rollback]" >&2
    exit 1
    ;;
esac
