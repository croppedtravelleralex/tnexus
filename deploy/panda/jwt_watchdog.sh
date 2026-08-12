#!/usr/bin/env bash
# JWT 看门狗：检测 NewAPI→gateway 鉴权是否失效，失效或临近过期时立即刷新。
#
# 背景：GATEWAY_AUTH_KEY 是 gateway 的 session JWT（约 24h）。此前 cron 每天 04:17
# 刷新一次，与过期时刻几乎重合；且 gateway 重启会使旧 session 失效，导致 NewAPI
# 渠道 115 出现成片 "401 invalid session"（占 7d 失败的 2/3）。
set -uo pipefail

ENV_FILE="${ENV_FILE:-/opt/tnexus/.env}"
GATEWAY="${GATEWAY:-http://127.0.0.1:8014}"
CHANNEL_ID="${CHANNEL_ID:-115}"
# 剩余有效期低于该秒数即提前刷新（默认 8h）
MIN_TTL="${MIN_TTL:-28800}"
REFRESH="${REFRESH:-/root/TNexus/deploy/panda/refresh_upstream_jwt.sh}"

ts() { date '+%Y-%m-%dT%H:%M:%S%z'; }
log() { echo "$(ts) [jwt-watchdog] $*"; }

jwt_ttl() {
  python3 - "$1" <<'PY'
import base64, json, sys, time
tok = sys.argv[1]
parts = tok.split(".")
if len(parts) != 3:
    print(-1); raise SystemExit
try:
    p = parts[1] + "=" * (-len(parts[1]) % 4)
    exp = json.loads(base64.urlsafe_b64decode(p)).get("exp")
    print(int(exp) - int(time.time()) if exp else -1)
except Exception:
    print(-1)
PY
}

probe() {
  curl -sS -o /dev/null -w '%{http_code}' --max-time 10 \
    -H "Authorization: Bearer $1" "$GATEWAY/v1/models" 2>/dev/null || echo 000
}

newapi_channel_key() {
  docker exec new-api-postgres psql -U newapi -d new-api -tAc \
    "SELECT key FROM channels WHERE id=$CHANNEL_ID" 2>/dev/null | tr -d '\r\n'
}

if ! curl -fsS -o /dev/null --max-time 5 "$GATEWAY/health" 2>/dev/null; then
  log "gateway unhealthy — skip (will retry next run)"
  exit 0
fi

ENV_KEY=$(grep -E '^GATEWAY_AUTH_KEY=' "$ENV_FILE" | cut -d= -f2- | tr -d '\r\n')
CH_KEY=$(newapi_channel_key)

NEED_REFRESH=0
REASONS=""

if [[ -z "$ENV_KEY" ]]; then
  NEED_REFRESH=1; REASONS="${REASONS}env_key_missing "
else
  TTL=$(jwt_ttl "$ENV_KEY")
  log "env_key ttl=${TTL}s"
  if [[ "$TTL" -lt "$MIN_TTL" ]]; then
    NEED_REFRESH=1; REASONS="${REASONS}ttl_low(${TTL}s) "
  fi
  CODE=$(probe "$ENV_KEY")
  log "env_key probe /v1/models -> $CODE"
  [[ "$CODE" == "401" ]] && { NEED_REFRESH=1; REASONS="${REASONS}env_key_401 "; }
fi

# NewAPI 实际使用的是渠道里存的 key，必须与 gateway 同源且有效
if [[ -z "$CH_KEY" ]]; then
  log "WARN cannot read channel $CHANNEL_ID key from NewAPI"
elif [[ "$CH_KEY" != "$ENV_KEY" ]]; then
  NEED_REFRESH=1; REASONS="${REASONS}channel_key_mismatch "
else
  CODE=$(probe "$CH_KEY")
  log "channel_key probe /v1/models -> $CODE"
  [[ "$CODE" == "401" ]] && { NEED_REFRESH=1; REASONS="${REASONS}channel_key_401 "; }
fi

if [[ "$NEED_REFRESH" -eq 0 ]]; then
  log "OK no refresh needed"
  exit 0
fi

log "REFRESHING reasons: $REASONS"
if bash "$REFRESH"; then
  NEW_KEY=$(grep -E '^GATEWAY_AUTH_KEY=' "$ENV_FILE" | cut -d= -f2- | tr -d '\r\n')
  NEW_CH=$(newapi_channel_key)
  CODE=$(probe "$NEW_KEY")
  log "after refresh: ttl=$(jwt_ttl "$NEW_KEY")s probe=$CODE channel_synced=$([[ "$NEW_KEY" == "$NEW_CH" ]] && echo yes || echo NO)"
  [[ "$CODE" == "200" && "$NEW_KEY" == "$NEW_CH" ]] && exit 0
  log "ERROR refresh completed but verification failed"
  exit 1
fi

log "ERROR refresh script failed"
exit 1
