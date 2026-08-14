#!/usr/bin/env bash
# NewAPI 生图渠道：token 分组 "tnexus-imagine" → grok2api-rs :8000 /v1/images/generations（Lite 默认）。
# 与 OCR（tnexus-ocr）、GPT 生图（tnexus-dedicated ch115）隔离。
#
#   bash newapi_tnexus_imagine.sh apply      # 建分组 + 渠道 + token + 按次定价
#   bash newapi_tnexus_imagine.sh sync-key   # 从 GROK_GATEWAY_AUTH_KEY 同步渠道 key
#   bash newapi_tnexus_imagine.sh status
#   bash newapi_tnexus_imagine.sh rollback
#
# 前置：宿主 :8000 Lite 生图冒烟通过（scripts/panda_grok_imagine_smoke.sh）。
set -euo pipefail

ENV_FILE="${ENV_FILE:-/opt/tnexus/.env}"
PG_CONTAINER="${PG_CONTAINER:-new-api-postgres}"
DB_USER="${NEWAPI_DB_USER:-newapi}"
DB_NAME="${NEWAPI_DB_NAME:-new-api}"
IMAGINE_GROUP="${TNEXUS_IMAGINE_GROUP:-tnexus-imagine}"
CHANNEL_NAME="${TNEXUS_IMAGINE_CHANNEL:-tnexus-imagine}"
TOKEN_NAME="${TNEXUS_IMAGINE_TOKEN:-tnexus-imagine-key}"
BASE_URL="${TNEXUS_IMAGINE_BASE:-http://host.docker.internal:8000}"
MODELS="${TNEXUS_IMAGINE_MODELS:-grok-imagine-lite}"
GROUP_RATIO="${TNEXUS_IMAGINE_GROUP_RATIO:-1.0}"
MODEL_PRICE="${TNEXUS_IMAGINE_MODEL_PRICE:-0.05}"
TOKEN_USER_ID="${TNEXUS_IMAGINE_TOKEN_USER_ID:-1}"
IMAGINE_HEALTH_URL="${TNEXUS_IMAGINE_HEALTH_URL:-http://127.0.0.1:8000/readyz}"

sql_escape() {
  printf '%s' "$1" | sed "s/'/''/g"
}

psql_exec() {
  docker exec "$PG_CONTAINER" psql -U "$DB_USER" -d "$DB_NAME" -v ON_ERROR_STOP=1 -c "$1"
}

psql_scalar() {
  docker exec "$PG_CONTAINER" psql -U "$DB_USER" -d "$DB_NAME" -t -A -v ON_ERROR_STOP=1 -c "$1" | tr -d '[:space:]'
}

load_imagine_key() {
  if [[ ! -f "$ENV_FILE" ]]; then
    echo "missing $ENV_FILE" >&2
    exit 1
  fi
  IMAGINE_KEY=$(grep '^GROK_GATEWAY_AUTH_KEY=' "$ENV_FILE" | cut -d= -f2- || true)
  if [[ -z "$IMAGINE_KEY" ]]; then
    echo "GROK_GATEWAY_AUTH_KEY missing in $ENV_FILE" >&2
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
  IF raw IS NULL THEN obj := '{}'::jsonb; ELSE obj := raw::jsonb; END IF;
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
  IF raw IS NULL THEN obj := '{}'::jsonb; ELSE obj := raw::jsonb; END IF;
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
  IF raw IS NULL THEN RETURN; END IF;
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
  load_imagine_key
  key_esc=$(sql_escape "$IMAGINE_KEY")
  base_esc=$(sql_escape "$BASE_URL")
  psql_exec "UPDATE channels SET key = '${key_esc}', base_url = '${base_esc}', status = 1 WHERE name = '${CHANNEL_NAME}';"
}

upsert_channel() {
  local key_esc base_esc models_esc created existing_id
  load_imagine_key
  key_esc=$(sql_escape "$IMAGINE_KEY")
  base_esc=$(sql_escape "$BASE_URL")
  models_esc=$(sql_escape "$MODELS")
  created=$(date +%s)
  existing_id=$(psql_scalar "SELECT id FROM channels WHERE name = '${CHANNEL_NAME}' LIMIT 1;")

  if [[ -n "$existing_id" ]]; then
    echo "==> update channel id=$existing_id ($CHANNEL_NAME)"
    psql_exec "UPDATE channels SET key = '${key_esc}', base_url = '${base_esc}', models = '${models_esc}', \"group\" = '${IMAGINE_GROUP}', weight = 100, status = 1 WHERE id = ${existing_id};"
  else
    echo "==> insert channel $CHANNEL_NAME → $BASE_URL (group=${IMAGINE_GROUP})"
    psql_exec "INSERT INTO channels (type, key, status, name, weight, created_time, base_url, models, \"group\", priority, auto_ban) VALUES (1, '${key_esc}', 1, '${CHANNEL_NAME}', 100, ${created}, '${base_esc}', '${models_esc}', '${IMAGINE_GROUP}', 100, 0);"
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
      VALUES ('${IMAGINE_GROUP}', '${model}', ${CHANNEL_ID}, true, 100, 100)
      ON CONFLICT (\"group\", model, channel_id) DO UPDATE
      SET enabled = EXCLUDED.enabled, priority = EXCLUDED.priority, weight = EXCLUDED.weight;"
  done
}

upsert_pricing() {
  local model
  IFS=',' read -ra MODEL_ARR <<<"$MODELS"
  for model in "${MODEL_ARR[@]}"; do
    model=$(echo "$model" | xargs)
    [[ -z "$model" ]] && continue
    echo "==> ModelPrice ${model} = ${MODEL_PRICE} USD/次"
    merge_option_json_number "ModelPrice" "$model" "$MODEL_PRICE"
  done
}

remove_pricing() {
  local model
  IFS=',' read -ra MODEL_ARR <<<"$MODELS"
  for model in "${MODEL_ARR[@]}"; do
    model=$(echo "$model" | xargs)
    [[ -z "$model" ]] && continue
    remove_option_json_key "ModelPrice" "$model"
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
    psql_exec "UPDATE tokens SET \"group\" = '${IMAGINE_GROUP}', status = 1, unlimited_quota = true WHERE name = '${name_esc}' AND deleted_at IS NULL;"
    return
  fi

  token_key=$(gen_token_key)
  key_esc=$(sql_escape "$token_key")
  echo "==> create token $TOKEN_NAME (group=${IMAGINE_GROUP})"
  psql_exec "INSERT INTO tokens (user_id, key, status, name, created_time, remain_quota, unlimited_quota, \"group\")
    VALUES (${TOKEN_USER_ID}, '${key_esc}', 1, '${name_esc}', ${created}, 0, true, '${IMAGINE_GROUP}');"
  TOKEN_KEY="$token_key"
}

preflight_container_reachability() {
  if ! docker ps --format '{{.Names}}' | grep -qx 'new-api'; then
    echo "==> skip 连通性预检（new-api 容器不在本机）"
    return 0
  fi
  if docker exec new-api sh -c "wget -q -T 8 -O- ${BASE_URL}/readyz" >/dev/null 2>&1; then
    echo "==> 预检通过：new-api 容器可达 ${BASE_URL}"
    return 0
  fi
  echo "new-api 容器无法连到 ${BASE_URL}" >&2
  exit 1
}

apply_imagine() {
  if ! curl -fsS -o /dev/null --max-time 5 "$IMAGINE_HEALTH_URL"; then
    echo "grok2api-rs :8000 not healthy ($IMAGINE_HEALTH_URL) — abort" >&2
    exit 1
  fi
  preflight_container_reachability

  echo "==> register group ${IMAGINE_GROUP} (ratio ${GROUP_RATIO})"
  merge_option_json "UserUsableGroups" "$IMAGINE_GROUP" "$MODELS"
  merge_option_json_number "GroupRatio" "$IMAGINE_GROUP" "$GROUP_RATIO"

  upsert_channel
  upsert_abilities
  upsert_pricing
  upsert_token

  if docker ps --format '{{.Names}}' | grep -qx 'new-api'; then
    echo "==> restart new-api"
    docker restart new-api >/dev/null
    for _ in $(seq 1 20); do
      if curl -fsS -o /dev/null --max-time 2 http://127.0.0.1:8081/api/status 2>/dev/null; then
        break
      fi
      sleep 2
    done
  fi

  echo ""
  echo "==> Imagine channel ready"
  echo "    group:   ${IMAGINE_GROUP} (ratio ${GROUP_RATIO})"
  echo "    channel: ${CHANNEL_NAME} (id=${CHANNEL_ID}) → ${BASE_URL}"
  echo "    model:   ${MODELS} @ \$${MODEL_PRICE}/次"
  echo "    token:   ${TOKEN_KEY}"
  echo ""
  echo "冒烟："
  echo "  curl -sS http://127.0.0.1:8081/v1/images/generations \\"
  echo "    -H 'Authorization: Bearer ${TOKEN_KEY}' \\"
  echo "    -H 'Content-Type: application/json' \\"
  echo "    -d '{\"model\":\"grok-imagine-lite\",\"prompt\":\"a red fox\",\"n\":1,\"response_format\":\"url\",\"size\":\"1024x1024\"}'"
  status_imagine
}

rollback_imagine() {
  echo "==> rollback imagine channel"
  remove_option_json_key "UserUsableGroups" "$IMAGINE_GROUP"
  remove_option_json_key "GroupRatio" "$IMAGINE_GROUP"
  remove_pricing

  local ch_id
  ch_id=$(psql_scalar "SELECT id FROM channels WHERE name = '${CHANNEL_NAME}' LIMIT 1;" || true)
  if [[ -n "$ch_id" ]]; then
    psql_exec "DELETE FROM abilities WHERE channel_id = ${ch_id};"
    psql_exec "DELETE FROM channels WHERE id = ${ch_id};"
  fi
  psql_exec "UPDATE tokens SET status = 2 WHERE name = '$(sql_escape "$TOKEN_NAME")' AND deleted_at IS NULL;"
  echo "rollback done"
}

status_imagine() {
  echo "==> channel"
  docker exec "$PG_CONTAINER" psql -U "$DB_USER" -d "$DB_NAME" \
    -c "SELECT id, name, \"group\", weight, base_url, status FROM channels WHERE name = '${CHANNEL_NAME}';"
}

case "${1:-apply}" in
  apply|"") apply_imagine ;;
  sync-key)
    sync_channel_key
    echo "synced ${CHANNEL_NAME} key from GROK_GATEWAY_AUTH_KEY"
    ;;
  status) status_imagine ;;
  rollback) rollback_imagine ;;
  *)
    echo "usage: $0 [apply|sync-key|status|rollback]" >&2
    exit 1
    ;;
esac
