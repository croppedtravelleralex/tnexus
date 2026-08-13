#!/usr/bin/env bash
# NewAPI OCR 渠道：token 分组 "tnexus-ocr" → grok2api-rs :8000（grok-vision-ocr），按次 $0.01。
# 与生图渠道（tnexus / tnexus-dedicated）完全隔离，不影响 ch114/115。
#
#   bash newapi_tnexus_ocr.sh apply      # 建分组 + 渠道 + token + 按次定价
#   bash newapi_tnexus_ocr.sh sync-key   # 从 GROK_GATEWAY_AUTH_KEY 重新同步渠道 key
#   bash newapi_tnexus_ocr.sh status     # 查看渠道 / 分组 / 定价 / token
#   bash newapi_tnexus_ocr.sh rollback   # 移除本脚本创建的全部配置
#
# 与生图渠道的两点关键差异：
#   1. key 用 GROK_GATEWAY_AUTH_KEY（静态密钥），不是每日轮换的 GATEWAY_AUTH_KEY JWT，
#      所以 refresh_upstream_jwt.sh 的 sync-key 不覆盖它。
#   2. 分组倍率固定 1.0：tnexus 组是 0.1，复用会让 $0.01/次 实收变成 $0.001。
set -euo pipefail

ENV_FILE="${ENV_FILE:-/opt/tnexus/.env}"
PG_CONTAINER="${PG_CONTAINER:-new-api-postgres}"
DB_USER="${NEWAPI_DB_USER:-newapi}"
DB_NAME="${NEWAPI_DB_NAME:-new-api}"
OCR_GROUP="${TNEXUS_OCR_GROUP:-tnexus-ocr}"
CHANNEL_NAME="${TNEXUS_OCR_CHANNEL:-tnexus-ocr}"
TOKEN_NAME="${TNEXUS_OCR_TOKEN:-tnexus-ocr-key}"
BASE_URL="${TNEXUS_OCR_BASE:-http://host.docker.internal:8000}"
MODELS="${TNEXUS_OCR_MODELS:-grok-vision-ocr}"
GROUP_RATIO="${TNEXUS_OCR_GROUP_RATIO:-1.0}"
# NewAPI 按次计费：配额消耗 = 固定价(USD) × 分组倍率 × 500000。
MODEL_PRICE="${TNEXUS_OCR_MODEL_PRICE:-0.01}"
TOKEN_USER_ID="${TNEXUS_OCR_TOKEN_USER_ID:-1}"
OCR_HEALTH_URL="${TNEXUS_OCR_HEALTH_URL:-http://127.0.0.1:8000/readyz}"

sql_escape() {
  printf '%s' "$1" | sed "s/'/''/g"
}

psql_exec() {
  docker exec "$PG_CONTAINER" psql -U "$DB_USER" -d "$DB_NAME" -v ON_ERROR_STOP=1 -c "$1"
}

psql_scalar() {
  docker exec "$PG_CONTAINER" psql -U "$DB_USER" -d "$DB_NAME" -t -A -v ON_ERROR_STOP=1 -c "$1" | tr -d '[:space:]'
}

load_ocr_key() {
  if [[ ! -f "$ENV_FILE" ]]; then
    echo "missing $ENV_FILE" >&2
    exit 1
  fi
  OCR_KEY=$(grep '^GROK_GATEWAY_AUTH_KEY=' "$ENV_FILE" | cut -d= -f2- || true)
  if [[ -z "$OCR_KEY" ]]; then
    echo "GROK_GATEWAY_AUTH_KEY missing in $ENV_FILE — grok-deploy.sh 未跑过？" >&2
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
  load_ocr_key
  key_esc=$(sql_escape "$OCR_KEY")
  base_esc=$(sql_escape "$BASE_URL")
  psql_exec "UPDATE channels SET key = '${key_esc}', base_url = '${base_esc}', status = 1 WHERE name = '${CHANNEL_NAME}';"
}

upsert_channel() {
  local key_esc base_esc models_esc created existing_id
  load_ocr_key
  key_esc=$(sql_escape "$OCR_KEY")
  base_esc=$(sql_escape "$BASE_URL")
  models_esc=$(sql_escape "$MODELS")
  created=$(date +%s)
  existing_id=$(psql_scalar "SELECT id FROM channels WHERE name = '${CHANNEL_NAME}' LIMIT 1;")

  if [[ -n "$existing_id" ]]; then
    echo "==> update channel id=$existing_id ($CHANNEL_NAME)"
    psql_exec "UPDATE channels SET key = '${key_esc}', base_url = '${base_esc}', models = '${models_esc}', \"group\" = '${OCR_GROUP}', weight = 100, status = 1 WHERE id = ${existing_id};"
  else
    echo "==> insert channel $CHANNEL_NAME → $BASE_URL (group=${OCR_GROUP})"
    psql_exec "INSERT INTO channels (type, key, status, name, weight, created_time, base_url, models, \"group\", priority, auto_ban) VALUES (1, '${key_esc}', 1, '${CHANNEL_NAME}', 100, ${created}, '${base_esc}', '${models_esc}', '${OCR_GROUP}', 100, 0);"
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
      VALUES ('${OCR_GROUP}', '${model}', ${CHANNEL_ID}, true, 100, 100)
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
    psql_exec "UPDATE tokens SET \"group\" = '${OCR_GROUP}', status = 1, unlimited_quota = true WHERE name = '${name_esc}' AND deleted_at IS NULL;"
    return
  fi

  token_key=$(gen_token_key)
  key_esc=$(sql_escape "$token_key")
  echo "==> create token $TOKEN_NAME (group=${OCR_GROUP})"
  psql_exec "INSERT INTO tokens (user_id, key, status, name, created_time, remain_quota, unlimited_quota, \"group\")
    VALUES (${TOKEN_USER_ID}, '${key_esc}', 1, '${name_esc}', ${created}, 0, true, '${OCR_GROUP}');"
  TOKEN_KEY="$token_key"
}

# NewAPI 经 host.docker.internal 回连宿主端口，而 UFW 默认 DROP：:8000 若没按来源
# 网段放行，表现是 NewAPI 侧「do request failed / connect: connection timed out」，
# 而宿主本地 curl 一切正常——极易误判成 OCR 服务故障。此处提前拦下。
preflight_container_reachability() {
  local nets subnet
  if ! docker ps --format '{{.Names}}' | grep -qx 'new-api'; then
    echo "==> skip 连通性预检（new-api 容器不在本机）"
    return 0
  fi
  if docker exec new-api sh -c "wget -q -T 8 -O- ${BASE_URL}/readyz" >/dev/null 2>&1; then
    echo "==> 预检通过：new-api 容器可达 ${BASE_URL}"
    return 0
  fi

  echo "new-api 容器无法连到 ${BASE_URL}（UFW 未放行 docker 网段 → :8000）" >&2
  echo "在本机执行以下命令后重试：" >&2
  nets=$(docker inspect new-api --format '{{range $k,$v := .NetworkSettings.Networks}}{{$k}} {{end}}')
  for n in $nets; do
    subnet=$(docker network inspect "$n" --format '{{range .IPAM.Config}}{{.Subnet}}{{end}}' 2>/dev/null)
    [[ -n "$subnet" ]] && echo "  ufw allow from ${subnet} to any port 8000 proto tcp comment 'newapi to grok2api-rs 8000'" >&2
  done
  echo "  ufw allow from 172.17.0.0/16 to any port 8000 proto tcp comment 'docker0 to grok2api-rs 8000'" >&2
  exit 1
}

apply_ocr() {
  if ! curl -fsS -o /dev/null --max-time 5 "$OCR_HEALTH_URL"; then
    echo "grok2api-rs :8000 not healthy ($OCR_HEALTH_URL) — abort" >&2
    exit 1
  fi
  preflight_container_reachability

  echo "==> register group ${OCR_GROUP} (ratio ${GROUP_RATIO})"
  merge_option_json "UserUsableGroups" "$OCR_GROUP" "$MODELS"
  merge_option_json_number "GroupRatio" "$OCR_GROUP" "$GROUP_RATIO"

  upsert_channel
  upsert_abilities
  upsert_pricing
  upsert_token

  if docker ps --format '{{.Names}}' | grep -qx 'new-api'; then
    echo "==> restart new-api (reload options)"
    docker restart new-api >/dev/null
    for _ in $(seq 1 20); do
      if curl -fsS -o /dev/null --max-time 2 http://127.0.0.1:8081/api/status 2>/dev/null; then
        break
      fi
      sleep 2
    done
  fi

  echo ""
  echo "==> OCR channel ready"
  echo "    group:   ${OCR_GROUP} (ratio ${GROUP_RATIO})"
  echo "    channel: ${CHANNEL_NAME} (id=${CHANNEL_ID}) → ${BASE_URL}"
  echo "    model:   ${MODELS} @ \$${MODEL_PRICE}/次"
  echo "    token:   ${TOKEN_KEY}"
  echo ""
  echo "冒烟（图片换成真实 base64 data URI）："
  echo "  curl -sS http://127.0.0.1:8081/v1/chat/completions \\"
  echo "    -H 'Authorization: Bearer ${TOKEN_KEY}' \\"
  echo "    -H 'Content-Type: application/json' \\"
  echo "    -d '{\"model\":\"grok-vision-ocr\",\"stream\":false,\"messages\":[{\"role\":\"user\",\"content\":[{\"type\":\"image_url\",\"image_url\":{\"url\":\"data:image/png;base64,...\"}},{\"type\":\"text\",\"text\":\"提取图中文字\"}]}]}'"
  echo ""
  echo "计费校验（期望 quota ≈ ${MODEL_PRICE} × ${GROUP_RATIO} × 500000）："
  echo "  docker exec ${PG_CONTAINER} psql -U ${DB_USER} -d ${DB_NAME} -c \\"
  echo "    \"SELECT model_name, quota, use_time FROM logs ORDER BY id DESC LIMIT 3;\""
  echo ""
  status_ocr
}

rollback_ocr() {
  echo "==> rollback OCR channel"
  remove_option_json_key "UserUsableGroups" "$OCR_GROUP"
  remove_option_json_key "GroupRatio" "$OCR_GROUP"
  remove_pricing

  local ch_id
  ch_id=$(psql_scalar "SELECT id FROM channels WHERE name = '${CHANNEL_NAME}' LIMIT 1;" || true)
  if [[ -n "$ch_id" ]]; then
    psql_exec "DELETE FROM abilities WHERE channel_id = ${ch_id};"
    psql_exec "DELETE FROM channels WHERE id = ${ch_id};"
  fi

  psql_exec "UPDATE tokens SET status = 2 WHERE name = '$(sql_escape "$TOKEN_NAME")' AND deleted_at IS NULL;"
  echo "rollback done（生图渠道 ch114/115 未受影响）"
}

status_ocr() {
  echo "==> options"
  docker exec "$PG_CONTAINER" psql -U "$DB_USER" -d "$DB_NAME" \
    -c "SELECT key, value FROM options WHERE key IN ('UserUsableGroups','GroupRatio');"
  echo "==> ModelPrice（本渠道模型）"
  docker exec "$PG_CONTAINER" psql -U "$DB_USER" -d "$DB_NAME" \
    -c "SELECT value::jsonb -> '${MODELS}' AS ocr_price FROM options WHERE key = 'ModelPrice';"
  echo "==> channel"
  docker exec "$PG_CONTAINER" psql -U "$DB_USER" -d "$DB_NAME" \
    -c "SELECT id, name, \"group\", weight, base_url, status FROM channels WHERE name = '${CHANNEL_NAME}';"
  echo "==> abilities"
  docker exec "$PG_CONTAINER" psql -U "$DB_USER" -d "$DB_NAME" \
    -c "SELECT \"group\", model, channel_id, enabled, weight FROM abilities WHERE \"group\" = '${OCR_GROUP}';"
  echo "==> token"
  docker exec "$PG_CONTAINER" psql -U "$DB_USER" -d "$DB_NAME" \
    -c "SELECT id, name, \"group\", status, unlimited_quota FROM tokens WHERE name = '$(sql_escape "$TOKEN_NAME")' AND deleted_at IS NULL;"
}

case "${1:-apply}" in
  apply|"") apply_ocr ;;
  sync-key)
    sync_channel_key
    echo "synced ${CHANNEL_NAME} key from GROK_GATEWAY_AUTH_KEY"
    ;;
  status) status_ocr ;;
  rollback) rollback_ocr ;;
  *)
    echo "usage: $0 [apply|sync-key|status|rollback]" >&2
    exit 1
    ;;
esac
