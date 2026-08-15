#!/usr/bin/env bash
# 从 NewAPI :8081 测 TNexus 能力（不碰 生图 灰度 ch84/114）。
#   1. grok-chat / grok-vision-ocr  →  tnexus-ocr  token → grok2api-rs :8000
#   2. gpt-image-2                  →  tnexus      token → gateway :8014
# 用法（Panda）：bash /root/TNexus/scripts/panda_newapi_tnexus_e2e.sh
set -uo pipefail

ROOT="${TNEXUS_ROOT:-/root/TNexus}"
NEWAPI="${NEWAPI_BASE:-http://127.0.0.1:8081}"
FIXTURE="${OCR_FIXTURE:-$ROOT/tests/grok_golden/ocr_fixture_tnexus.png}"
OCR_MARK="TNEXUS-OCR-OK"
PG="docker exec new-api-postgres psql -U newapi -d new-api"

PASS=0
FAIL=0
SKIP=0
RESULTS=()

log() { printf '%s\n' "$*"; }
ok() { PASS=$((PASS + 1)); RESULTS+=("PASS  $1"); log "PASS  $1"; }
bad() { FAIL=$((FAIL + 1)); RESULTS+=("FAIL  $1 — $2"); log "FAIL  $1 — $2"; }
skip() { SKIP=$((SKIP + 1)); RESULTS+=("SKIP  $1 — $2"); log "SKIP  $1 — $2"; }

q() { $PG -t -A -v ON_ERROR_STOP=1 -c "$1" | tr -d '[:space:]'; }

token_key() {
  q "SELECT key FROM tokens WHERE name='$1' AND deleted_at IS NULL LIMIT 1;"
}

contains() {
  python3 -c 'import re,sys
hay=re.sub(r"\s+","",sys.stdin.read()).lower()
needle=re.sub(r"\s+","",sys.argv[1]).lower()
sys.exit(0 if needle in hay else 1)' "$1"
}

json_get() {
  python3 -c 'import json,sys
d=json.load(sys.stdin)
def walk(o,p):
  if not p: return o
  k=p[0]
  if isinstance(o,dict): return walk(o.get(k), p[1:])
  if isinstance(o,list) and k.isdigit(): return walk(o[int(k)], p[1:])
  return None
v=walk(d, sys.argv[1].split("."))
if v is None: sys.exit(1)
if isinstance(v,(dict,list)): print(json.dumps(v,ensure_ascii=False))
else: print(v)' "$1"
}

b64_image() {
  python3 - "$FIXTURE" <<'PY'
import base64, pathlib, sys
print(base64.b64encode(pathlib.Path(sys.argv[1]).read_bytes()).decode("ascii"))
PY
}

prepare() {
  log "==> prepare NewAPI TNexus channels"

  # 专用生图渠道：status=2 / ability 关闭会让 tnexus 组完全打不到 :8014
  $PG -v ON_ERROR_STOP=1 -c "
    UPDATE channels SET status = 1 WHERE id = 115 AND name = 'tnexus-dedicated';
    UPDATE abilities SET enabled = true
      WHERE channel_id = 115 AND model = 'gpt-image-2' AND \"group\" = 'tnexus';
  " >/dev/null

  # grok-chat 接到已有 tnexus-ocr 渠道（同一 :8000 + GROK_GATEWAY_AUTH_KEY）
  $PG -v ON_ERROR_STOP=1 -c "
    UPDATE channels
      SET models = 'grok-vision-ocr,grok-chat', status = 1
      WHERE id = 117 AND name = 'tnexus-ocr';
    INSERT INTO abilities (\"group\", model, channel_id, enabled, priority, weight)
      VALUES ('tnexus-ocr', 'grok-chat', 117, true, 100, 100)
      ON CONFLICT (\"group\", model, channel_id) DO UPDATE
        SET enabled = true, priority = 100, weight = 100;
  " >/dev/null

  docker exec -i new-api-postgres psql -U newapi -d new-api -v ON_ERROR_STOP=1 <<'SQL' >/dev/null
DO $$
DECLARE
  raw text;
  obj jsonb;
BEGIN
  SELECT value INTO raw FROM options WHERE key = 'UserUsableGroups' LIMIT 1;
  IF raw IS NULL THEN obj := '{}'::jsonb; ELSE obj := raw::jsonb; END IF;
  obj := obj || jsonb_build_object('tnexus-ocr', 'grok-vision-ocr,grok-chat');
  UPDATE options SET value = obj::text WHERE key = 'UserUsableGroups';

  -- 对话按 token 计费（与 grok-2 同档）；不配价格时 NewAPI 直接 400 model_price_error
  SELECT value INTO raw FROM options WHERE key = 'ModelRatio' LIMIT 1;
  IF raw IS NULL THEN obj := '{}'::jsonb; ELSE obj := raw::jsonb; END IF;
  obj := obj || jsonb_build_object('grok-chat', 1::numeric);
  IF EXISTS (SELECT 1 FROM options WHERE key = 'ModelRatio') THEN
    UPDATE options SET value = obj::text WHERE key = 'ModelRatio';
  ELSE
    INSERT INTO options (key, value) VALUES ('ModelRatio', obj::text);
  END IF;
END $$;
SQL

  if [[ -f /root/TNexus/deploy/panda/newapi_tnexus_dedicated.sh ]]; then
    bash /root/TNexus/deploy/panda/newapi_tnexus_dedicated.sh sync-key >/dev/null || true
  fi
  if [[ -f /root/TNexus/deploy/panda/newapi_tnexus_ocr.sh ]]; then
    bash /root/TNexus/deploy/panda/newapi_tnexus_ocr.sh sync-key >/dev/null || true
  fi

  docker restart new-api >/dev/null
  local i
  for i in $(seq 1 20); do
    if curl -fsS -o /dev/null --max-time 2 "$NEWAPI/api/status" 2>/dev/null; then
      break
    fi
    sleep 2
  done
}

case_health() {
  if curl -fsS -m 5 "$NEWAPI/api/status" >/dev/null; then ok "NewAPI /api/status"; else bad "NewAPI /api/status" "down"; fi
  if docker exec new-api sh -c "wget -q -T 8 -O- http://host.docker.internal:8000/readyz" >/dev/null 2>&1; then
    ok "new-api → grok :8000"
  else
    bad "new-api → grok :8000" "timeout"
  fi
  if docker exec new-api sh -c "wget -q -T 8 -O- http://host.docker.internal:8014/health" >/dev/null 2>&1; then
    ok "new-api → gateway :8014"
  else
    bad "new-api → gateway :8014" "timeout"
  fi
}

case_models() {
  local ocr_tok img_tok body
  ocr_tok=$(token_key tnexus-ocr-key)
  img_tok=$(token_key tnexus-test-key)
  if [[ -z "$ocr_tok" ]]; then
    bad "OCR token /v1/models" "无 tnexus-ocr-key"
  else
    body=$(curl -sS -m 15 -H "Authorization: Bearer sk-${ocr_tok}" "$NEWAPI/v1/models" || true)
    if printf '%s' "$body" | contains 'grok-vision-ocr'; then ok "OCR token 可见 grok-vision-ocr"; else bad "OCR token 可见 grok-vision-ocr" "${body:0:300}"; fi
    if printf '%s' "$body" | contains 'grok-chat'; then ok "OCR token 可见 grok-chat"; else bad "OCR token 可见 grok-chat" "${body:0:300}"; fi
  fi
  if [[ -z "$img_tok" ]]; then
    bad "生图 token /v1/models" "无 tnexus-test-key"
  else
    body=$(curl -sS -m 15 -H "Authorization: Bearer sk-${img_tok}" "$NEWAPI/v1/models" || true)
    if printf '%s' "$body" | contains 'gpt-image-2'; then ok "生图 token 可见 gpt-image-2"; else bad "生图 token 可见 gpt-image-2" "${body:0:300}"; fi
  fi
}

call_chat() {
  local token="$1" model="$2" payload="$3" timeout="$4"
  curl -sS -m "$timeout" -w '\n%{http_code}' \
    -H "Authorization: Bearer sk-${token}" \
    -H 'Content-Type: application/json' \
    -d "$payload" \
    "$NEWAPI/v1/chat/completions" || true
}

case_grok_chat() {
  local tok body code json text start elapsed
  tok=$(token_key tnexus-ocr-key)
  if [[ -z "$tok" ]]; then skip "NewAPI grok-chat 200" "无 token"; return; fi
  start=$(date +%s)
  body=$(call_chat "$tok" grok-chat '{"model":"grok-chat","stream":false,"messages":[{"role":"user","content":"Reply with exactly PONG and nothing else."}]}' 45)
  elapsed=$(( $(date +%s) - start ))
  code=$(printf '%s' "$body" | tail -n1)
  json=$(printf '%s' "$body" | sed '$d')
  if [[ "$code" != "200" ]]; then
    bad "NewAPI grok-chat 200" "code=$code elapsed=${elapsed}s body=${json:0:500}"
    skip "NewAPI grok-chat PONG" "非 200"
    skip "NewAPI grok-chat 无 grok:render" "非 200"
    return
  fi
  ok "NewAPI grok-chat 200 (${elapsed}s)"
  text=$(printf '%s' "$json" | json_get choices.0.message.content 2>/dev/null || true)
  if printf '%s' "$text" | contains 'PONG'; then ok "NewAPI grok-chat PONG"; else bad "NewAPI grok-chat PONG" "content=${text:0:300}"; fi
  if printf '%s' "$json" | contains '<grok:render'; then bad "NewAPI grok-chat 无 grok:render" "仍含 markup"; else ok "NewAPI grok-chat 无 grok:render"; fi
}

case_grok_chat_stream() {
  local tok code n
  tok=$(token_key tnexus-ocr-key)
  if [[ -z "$tok" ]]; then skip "NewAPI grok-chat SSE" "无 token"; return; fi
  code=$(curl -sS -m 45 -o /tmp/newapi_grok_stream.txt -w '%{http_code}' \
    -H "Authorization: Bearer sk-${tok}" \
    -H 'Content-Type: application/json' \
    -d '{"model":"grok-chat","stream":true,"messages":[{"role":"user","content":"Reply with exactly PONG and nothing else."}]}' \
    "$NEWAPI/v1/chat/completions" || true)
  if [[ "$code" != "200" ]]; then
    bad "NewAPI grok-chat SSE 200" "code=$code body=$(head -c 300 /tmp/newapi_grok_stream.txt 2>/dev/null || true)"
    return
  fi
  ok "NewAPI grok-chat SSE 200"
  n=$(grep -c '^data:' /tmp/newapi_grok_stream.txt || true)
  if [[ "${n:-0}" -ge 1 ]]; then ok "NewAPI grok-chat SSE 含 data 帧 ($n)"; else bad "NewAPI grok-chat SSE 含 data 帧" "$(head -c 300 /tmp/newapi_grok_stream.txt)"; fi
}

case_ocr() {
  local tok b64 body code json text
  tok=$(token_key tnexus-ocr-key)
  if [[ -z "$tok" || ! -f "$FIXTURE" ]]; then skip "NewAPI OCR" "无 token/fixture"; return; fi
  b64=$(b64_image)
  body=$(call_chat "$tok" grok-vision-ocr "{\"model\":\"grok-vision-ocr\",\"stream\":false,\"messages\":[{\"role\":\"user\",\"content\":[{\"type\":\"image_url\",\"image_url\":{\"url\":\"data:image/png;base64,${b64}\"}},{\"type\":\"text\",\"text\":\"把看到的字写出来，不要解释。\"}]}]}" 90)
  code=$(printf '%s' "$body" | tail -n1)
  json=$(printf '%s' "$body" | sed '$d')
  if [[ "$code" != "200" ]]; then
    bad "NewAPI OCR 200" "code=$code body=${json:0:500}"
    skip "NewAPI OCR 识别标记" "非 200"
    return
  fi
  ok "NewAPI OCR 200"
  text=$(printf '%s' "$json" | json_get choices.0.message.content 2>/dev/null || true)
  if printf '%s' "$text" | contains "$OCR_MARK"; then ok "NewAPI OCR 识别 $OCR_MARK"; else bad "NewAPI OCR 识别 $OCR_MARK" "content=${text:0:400}"; fi
}

case_image() {
  local tok start elapsed code json url b64
  tok=$(token_key tnexus-test-key)
  if [[ -z "$tok" ]]; then skip "NewAPI gpt-image-2" "无 tnexus-test-key"; return; fi
  start=$(date +%s)
  json=$(curl -sS -m 240 -o /tmp/newapi_image.json -w '%{http_code}' \
    -H "Authorization: Bearer sk-${tok}" \
    -H 'Content-Type: application/json' \
    -d "{\"model\":\"gpt-image-2\",\"prompt\":\"tiny red apple on white background, test $(date +%s)\",\"n\":1,\"size\":\"256x256\",\"response_format\":\"b64_json\"}" \
    "$NEWAPI/v1/images/generations" || true)
  elapsed=$(( $(date +%s) - start ))
  code="$json"
  if [[ "$code" != "200" ]]; then
    bad "NewAPI gpt-image-2 200" "code=$code elapsed=${elapsed}s body=$(head -c 500 /tmp/newapi_image.json)"
    skip "NewAPI gpt-image-2 有图数据" "非 200"
    return
  fi
  ok "NewAPI gpt-image-2 200 (${elapsed}s)"
  if python3 - <<'PY'
import json,sys
d=json.load(open("/tmp/newapi_image.json"))
data=(d.get("data") or [None])[0] or {}
ok=bool(data.get("b64_json") or data.get("url"))
sys.exit(0 if ok else 1)
PY
  then
    ok "NewAPI gpt-image-2 有图数据"
  else
    bad "NewAPI gpt-image-2 有图数据" "$(head -c 400 /tmp/newapi_image.json)"
  fi
}

case_billing() {
  local ocr_q img_q
  ocr_q=$(q "SELECT quota FROM logs WHERE model_name='grok-vision-ocr' AND type=2 ORDER BY id DESC LIMIT 1;")
  img_q=$(q "SELECT quota FROM logs WHERE model_name='gpt-image-2' AND channel_id=115 AND type=2 ORDER BY id DESC LIMIT 1;")
  if [[ "$ocr_q" == "5000" ]]; then ok "OCR 计费 quota=5000"; else bad "OCR 计费 quota=5000" "got=${ocr_q:-empty}"; fi
  if [[ -n "$img_q" && "$img_q" != "0" ]]; then ok "生图 ch115 计费 quota=$img_q"; else skip "生图 ch115 计费" "本次无成功日志 quota=${img_q:-empty}"; fi
}

prepare
log "=== NewAPI → TNexus E2E ==="
case_health
case_models
case_grok_chat
case_grok_chat_stream
case_ocr
case_image
case_billing

TOTAL=$((PASS + FAIL))
PCT=0
[[ "$TOTAL" -gt 0 ]] && PCT=$((PASS * 100 / TOTAL))
log ""
log "=== 结果 $PASS passed / $FAIL failed / $SKIP skipped (${PCT}%) ==="
for r in "${RESULTS[@]}"; do log "  $r"; done
[[ "$TOTAL" -gt 0 && "$PCT" -ge 90 ]]
