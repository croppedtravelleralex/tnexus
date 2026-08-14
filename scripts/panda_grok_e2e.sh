#!/usr/bin/env bash
# Panda 生产 E2E：grok2api-rs :8000 / admin :8091 / NewAPI :8081 OCR。
# 覆盖本批能力（健康、额度 summary、对话 SSE、剥 markup、直连 OCR、NewAPI OCR）。
# 用法（Panda）：
#   bash /root/TNexus/scripts/panda_grok_e2e.sh
set -uo pipefail

ENV_FILE="${ENV_FILE:-/opt/tnexus/.env}"
ROOT="${TNEXUS_ROOT:-/root/TNexus}"
GROK="${GROK_BASE:-http://127.0.0.1:8000}"
ADMIN="${ADMIN_BASE:-http://127.0.0.1:8091}"
NEWAPI="${NEWAPI_BASE:-http://127.0.0.1:8081}"
FIXTURE="${OCR_FIXTURE:-$ROOT/tests/grok_golden/ocr_fixture_tnexus.png}"
OCR_MARK="TNEXUS-OCR-OK"

PASS=0
FAIL=0
SKIP=0
RESULTS=()

log() { printf '%s\n' "$*"; }
ok() { PASS=$((PASS + 1)); RESULTS+=("PASS  $1"); log "PASS  $1"; }
bad() { FAIL=$((FAIL + 1)); RESULTS+=("FAIL  $1 — $2"); log "FAIL  $1 — $2"; }
skip() { SKIP=$((SKIP + 1)); RESULTS+=("SKIP  $1 — $2"); log "SKIP  $1 — $2"; }

need_env() {
  if [[ ! -f "$ENV_FILE" ]]; then
    echo "missing $ENV_FILE" >&2
    exit 2
  fi
  set -a
  # shellcheck disable=SC1090
  source "$ENV_FILE"
  set +a
  GROK_KEY="${GROK_GATEWAY_AUTH_KEY:-}"
  ADMIN_USER="${GROK_ADMIN_USERNAME:-admin}"
  ADMIN_PASS="${GROK_ADMIN_PASSWORD:-}"
  if [[ -z "$GROK_KEY" ]]; then
    echo "GROK_GATEWAY_AUTH_KEY missing" >&2
    exit 2
  fi
}

ensure_fixture() {
  if [[ -f "$FIXTURE" ]]; then
    return 0
  fi
  local out="/tmp/ocr_fixture_tnexus.png"
  if python3 - <<'PY'
from pathlib import Path
try:
    from PIL import Image, ImageDraw
except ImportError:
    raise SystemExit(2)
img = Image.new("RGB", (480, 96), "white")
ImageDraw.Draw(img).text((24, 28), "TNEXUS-OCR-OK", fill=(0, 0, 0))
Path("/tmp/ocr_fixture_tnexus.png").write_bytes(b"")
img.save("/tmp/ocr_fixture_tnexus.png")
print("ok")
PY
  then
    FIXTURE="$out"
    return 0
  fi
  if command -v convert >/dev/null 2>&1; then
    convert -size 480x96 xc:white -pointsize 36 -fill black -gravity center \
      -annotate +0+0 "$OCR_MARK" "$out"
    FIXTURE="$out"
    return 0
  fi
  echo "no OCR fixture and cannot generate one" >&2
  exit 2
}

b64_image() {
  python3 - "$FIXTURE" <<'PY'
import base64, pathlib, sys
p = pathlib.Path(sys.argv[1])
print(base64.b64encode(p.read_bytes()).decode("ascii"))
PY
}

json_get() {
  python3 -c 'import json,sys; d=json.load(sys.stdin)
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

contains() {
  python3 -c 'import re,sys
hay=re.sub(r"\s+","",sys.stdin.read()).lower()
needle=re.sub(r"\s+","",sys.argv[1]).lower()
sys.exit(0 if needle in hay else 1)' "$1"
}

case_healthz() {
  local body code
  body=$(curl -sS -m 8 -w '\n%{http_code}' "$GROK/healthz" || true)
  code=$(printf '%s' "$body" | tail -n1)
  if [[ "$code" == "200" ]]; then ok "healthz 200"; else bad "healthz 200" "code=$code"; fi
}

case_readyz() {
  local body code json
  body=$(curl -sS -m 8 -w '\n%{http_code}' "$GROK/readyz" || true)
  code=$(printf '%s' "$body" | tail -n1)
  json=$(printf '%s' "$body" | sed '$d')
  if [[ "$code" != "200" ]]; then
    bad "readyz 200" "code=$code body=${json:0:300}"
    return
  fi
  ok "readyz 200"
  if printf '%s' "$json" | contains '"status"'; then
    ok "readyz 含 status"
  else
    bad "readyz 含 status" "$json"
  fi
  if printf '%s' "$json" | python3 -c 'import json,sys
d=json.load(sys.stdin)
keys=set(d.keys()) if isinstance(d,dict) else set()
# 新版带号池/额度指标；旧版只有 status。部署后应带至少一项观测字段。
need={"pool_size","quota_windows","pool_reconciled_at","credential_missing","quota_oldest_synced_at"}
sys.exit(0 if keys & need else 1)'; then
    ok "readyz 含号池/额度指标"
  else
    bad "readyz 含号池/额度指标" "$json"
  fi
}

case_models() {
  local body
  body=$(curl -sS -m 8 -H "Authorization: Bearer $GROK_KEY" "$GROK/v1/models" || true)
  if printf '%s' "$body" | contains 'grok-chat'; then
    ok "models 含 grok-chat"
  else
    bad "models 含 grok-chat" "${body:0:400}"
  fi
  if printf '%s' "$body" | contains 'grok-vision-ocr'; then
    ok "models 含 grok-vision-ocr"
  else
    bad "models 含 grok-vision-ocr" "${body:0:400}"
  fi
}

case_auth_401() {
  local code
  code=$(curl -sS -m 8 -o /tmp/e2e_unauth.json -w '%{http_code}' \
    -H 'Content-Type: application/json' \
    -d '{"model":"grok-chat","messages":[{"role":"user","content":"hi"}]}' \
    "$GROK/v1/chat/completions" || true)
  if [[ "$code" == "401" ]]; then ok "无 key 对话 401"; else bad "无 key 对话 401" "code=$code"; fi
}

case_admin() {
  if [[ -z "$ADMIN_PASS" ]]; then
    skip "admin login" "GROK_ADMIN_PASSWORD 未配置"
    skip "admin summary 200" "依赖 login"
    skip "admin remaining_fresh" "依赖 login"
    return
  fi
  local login code token summary
  login=$(curl -sS -m 8 -w '\n%{http_code}' -H 'Content-Type: application/json' \
    -d "{\"username\":\"${ADMIN_USER}\",\"password\":\"${ADMIN_PASS}\"}" \
    "$ADMIN/admin/auth/login" || true)
  code=$(printf '%s' "$login" | tail -n1)
  if [[ "$code" != "200" ]]; then
    bad "admin login" "code=$code"
    skip "admin summary 200" "login 失败"
    skip "admin remaining_fresh" "login 失败"
    return
  fi
  ok "admin login"
  token=$(printf '%s' "$login" | sed '$d' | json_get tokens.access_token || true)
  if [[ -z "$token" ]]; then
    bad "admin summary 200" "无 access_token"
    skip "admin remaining_fresh" "无 token"
    return
  fi
  summary=$(curl -sS -m 15 -w '\n%{http_code}' \
    -H "Authorization: Bearer $token" \
    "$ADMIN/admin/accounts/summary" || true)
  code=$(printf '%s' "$summary" | tail -n1)
  local sjson
  sjson=$(printf '%s' "$summary" | sed '$d')
  if [[ "$code" == "200" ]]; then
    ok "admin summary 200"
  else
    bad "admin summary 200" "code=$code body=${sjson:0:400}"
    skip "admin remaining_fresh" "summary 非 200"
    return
  fi
  if printf '%s' "$sjson" | contains 'remaining_fresh'; then
    ok "admin remaining_fresh"
  else
    bad "admin remaining_fresh" "${sjson:0:400}"
  fi
}

case_chat() {
  local start end elapsed body code json text
  start=$(date +%s)
  body=$(curl -sS -m 45 -w '\n%{http_code}' \
    -H "Authorization: Bearer $GROK_KEY" \
    -H 'Content-Type: application/json' \
    -d '{"model":"grok-chat","stream":false,"messages":[{"role":"user","content":"Reply with exactly PONG and nothing else."}]}' \
    "$GROK/v1/chat/completions" || true)
  end=$(date +%s)
  elapsed=$((end - start))
  code=$(printf '%s' "$body" | tail -n1)
  json=$(printf '%s' "$body" | sed '$d')
  if [[ "$code" != "200" ]]; then
    bad "对话非流式 200" "code=$code elapsed=${elapsed}s body=${json:0:500}"
    skip "对话返回 PONG" "非 200"
    skip "对话无 grok:render" "非 200"
    skip "对话耗时 <25s" "非 200"
    return
  fi
  ok "对话非流式 200"
  text=$(printf '%s' "$json" | json_get choices.0.message.content 2>/dev/null || true)
  if printf '%s' "$text" | contains 'PONG'; then
    ok "对话返回 PONG"
  else
    bad "对话返回 PONG" "content=${text:0:300}"
  fi
  if printf '%s' "$json" | contains '<grok:render'; then
    bad "对话无 grok:render" "仍含 markup"
  else
    ok "对话无 grok:render"
  fi
  if [[ "$elapsed" -lt 25 ]]; then
    ok "对话耗时 <25s (${elapsed}s)"
  else
    bad "对话耗时 <25s" "elapsed=${elapsed}s"
  fi
}

case_stream() {
  local start elapsed code n_data
  start=$(date +%s)
  code=$(curl -sS -m 45 -o /tmp/e2e_stream.txt -w '%{http_code}' \
    -H "Authorization: Bearer $GROK_KEY" \
    -H 'Content-Type: application/json' \
    -d '{"model":"grok-chat","stream":true,"messages":[{"role":"user","content":"Reply with exactly PONG and nothing else."}]}' \
    "$GROK/v1/chat/completions" || true)
  elapsed=$(( $(date +%s) - start ))
  if [[ "$code" != "200" ]]; then
    bad "对话 SSE 200" "code=$code elapsed=${elapsed}s body=$(head -c 400 /tmp/e2e_stream.txt 2>/dev/null || true)"
    skip "对话 SSE 含 data 帧" "非 200"
    skip "SSE 无 grok:render" "非 200"
    return
  fi
  ok "对话 SSE 200"
  n_data=$(grep -c '^data:' /tmp/e2e_stream.txt || true)
  if [[ "${n_data:-0}" -ge 1 ]]; then
    ok "对话 SSE 含 data 帧 ($n_data)"
  else
    bad "对话 SSE 含 data 帧" "$(head -c 400 /tmp/e2e_stream.txt)"
  fi
  if grep -q '<grok:render' /tmp/e2e_stream.txt; then
    bad "SSE 无 grok:render" "仍含 markup"
  else
    ok "SSE 无 grok:render"
  fi
}

case_ocr_direct() {
  local b64 body code text
  b64=$(b64_image)
  body=$(curl -sS -m 90 -w '\n%{http_code}' \
    -H "Authorization: Bearer $GROK_KEY" \
    -H 'Content-Type: application/json' \
    -d "{\"model\":\"grok-vision-ocr\",\"stream\":false,\"messages\":[{\"role\":\"user\",\"content\":[{\"type\":\"image_url\",\"image_url\":{\"url\":\"data:image/png;base64,${b64}\"}},{\"type\":\"text\",\"text\":\"把看到的字写出来，不要解释。\"}]}]}" \
    "$GROK/v1/chat/completions" || true)
  code=$(printf '%s' "$body" | tail -n1)
  local json
  json=$(printf '%s' "$body" | sed '$d')
  if [[ "$code" != "200" ]]; then
    bad "直连 OCR 200" "code=$code body=${json:0:600}"
    skip "直连 OCR 识别 TNEXUS-OCR-OK" "非 200"
    return
  fi
  ok "直连 OCR 200"
  text=$(printf '%s' "$json" | json_get choices.0.message.content 2>/dev/null || true)
  if printf '%s' "$text" | contains "$OCR_MARK"; then
    ok "直连 OCR 识别 $OCR_MARK"
  else
    bad "直连 OCR 识别 $OCR_MARK" "content=${text:0:400}"
  fi
}

newapi_token() {
  docker exec new-api-postgres psql -U newapi -d new-api -t -A -v ON_ERROR_STOP=1 \
    -c "SELECT key FROM tokens WHERE name='tnexus-ocr-key' AND deleted_at IS NULL LIMIT 1;" \
    | tr -d '[:space:]'
}

ensure_newapi_ocr() {
  if ! docker ps --format '{{.Names}}' | grep -qx 'new-api'; then
    return 1
  fi
  local n
  n=$(docker exec new-api-postgres psql -U newapi -d new-api -t -A \
    -c "SELECT count(*) FROM channels WHERE name='tnexus-ocr';" | tr -d '[:space:]')
  if [[ "${n:-0}" == "0" ]]; then
    bash "$ROOT/deploy/panda/newapi_tnexus_ocr.sh" apply
  else
    bash "$ROOT/deploy/panda/newapi_tnexus_ocr.sh" sync-key >/dev/null
  fi
}

case_ocr_newapi() {
  if ! ensure_newapi_ocr; then
    skip "NewAPI OCR 200" "new-api 容器不在"
    skip "NewAPI OCR 识别 TNEXUS-OCR-OK" "new-api 不在"
    skip "NewAPI OCR Bearer sk- 前缀" "new-api 不在"
    return
  fi
  local token b64 body code json text
  token=$(newapi_token)
  if [[ -z "$token" ]]; then
    bad "NewAPI OCR 200" "tokens 表无 tnexus-ocr-key"
    skip "NewAPI OCR 识别 TNEXUS-OCR-OK" "无 token"
    skip "NewAPI OCR Bearer sk- 前缀" "无 token"
    return
  fi
  b64=$(b64_image)
  body=$(curl -sS -m 90 -w '\n%{http_code}' \
    -H "Authorization: Bearer sk-${token}" \
    -H 'Content-Type: application/json' \
    -d "{\"model\":\"grok-vision-ocr\",\"stream\":false,\"messages\":[{\"role\":\"user\",\"content\":[{\"type\":\"image_url\",\"image_url\":{\"url\":\"data:image/png;base64,${b64}\"}},{\"type\":\"text\",\"text\":\"把看到的字写出来，不要解释。\"}]}]}" \
    "$NEWAPI/v1/chat/completions" || true)
  code=$(printf '%s' "$body" | tail -n1)
  json=$(printf '%s' "$body" | sed '$d')
  if [[ "$code" != "200" ]]; then
    # 部分 NewAPI 版本接受不带 sk- 的 key
    body=$(curl -sS -m 90 -w '\n%{http_code}' \
      -H "Authorization: Bearer ${token}" \
      -H 'Content-Type: application/json' \
      -d "{\"model\":\"grok-vision-ocr\",\"stream\":false,\"messages\":[{\"role\":\"user\",\"content\":[{\"type\":\"image_url\",\"image_url\":{\"url\":\"data:image/png;base64,${b64}\"}},{\"type\":\"text\",\"text\":\"把看到的字写出来，不要解释。\"}]}]}" \
      "$NEWAPI/v1/chat/completions" || true)
    code=$(printf '%s' "$body" | tail -n1)
    json=$(printf '%s' "$body" | sed '$d')
    if [[ "$code" == "200" ]]; then
      ok "NewAPI OCR 200"
      bad "NewAPI OCR Bearer sk- 前缀" "仅裸 key 成功"
    else
      bad "NewAPI OCR 200" "code=$code body=${json:0:700}"
      skip "NewAPI OCR 识别 TNEXUS-OCR-OK" "非 200"
      skip "NewAPI OCR Bearer sk- 前缀" "非 200"
      return
    fi
  else
    ok "NewAPI OCR 200"
    ok "NewAPI OCR Bearer sk- 前缀"
  fi
  text=$(printf '%s' "$json" | json_get choices.0.message.content 2>/dev/null || true)
  if printf '%s' "$text" | contains "$OCR_MARK"; then
    ok "NewAPI OCR 识别 $OCR_MARK"
  else
    bad "NewAPI OCR 识别 $OCR_MARK" "content=${text:0:400}"
  fi
}

case_public_page() {
  local code
  code=$(curl -sS -m 15 -o /dev/null -w '%{http_code}' https://tnexus.relai.asia/accounts || true)
  if [[ "$code" == "200" || "$code" == "302" ]]; then
    ok "公网 /accounts 可达 ($code)"
  else
    bad "公网 /accounts 可达" "code=$code"
  fi
}

case_imagine_lite() {
  local body code json
  body=$(curl -sS -m 150 -w '\n%{http_code}' \
    -H "Authorization: Bearer $GROK_KEY" \
    -H 'Content-Type: application/json' \
    -d '{"prompt":"a simple red circle on white background","n":1,"response_format":"url","size":"1024x1024"}' \
    "$GROK/v1/images/generations" || true)
  code=$(printf '%s' "$body" | tail -n1)
  json=$(printf '%s' "$body" | sed '$d')
  if [[ "$code" != "200" ]]; then
    bad "Lite 生图 200" "code=$code body=${json:0:600}"
    skip "Lite 生图含 url/b64" "非 200"
    return
  fi
  ok "Lite 生图 200"
  if printf '%s' "$json" | grep -qE '"url"|"b64_json"'; then
    ok "Lite 生图含 url/b64"
  else
    bad "Lite 生图含 url/b64" "${json:0:400}"
  fi
}

need_env
ensure_fixture

log "=== Grok E2E fixture=$FIXTURE ==="
case_healthz
case_readyz
case_models
case_auth_401
case_admin
case_chat
case_stream
case_ocr_direct
case_ocr_newapi
case_imagine_lite
case_public_page

TOTAL=$((PASS + FAIL))
PCT=0
if [[ "$TOTAL" -gt 0 ]]; then
  PCT=$((PASS * 100 / TOTAL))
fi

log ""
log "=== 结果 $PASS passed / $FAIL failed / $SKIP skipped (${PCT}% of executed) ==="
for r in "${RESULTS[@]}"; do log "  $r"; done

if [[ "$TOTAL" -eq 0 ]]; then
  exit 2
fi
if [[ "$PCT" -lt 90 ]]; then
  log "覆盖率 ${PCT}% < 90%"
  exit 1
fi
log "覆盖率 ${PCT}% ≥ 90%"
exit 0
