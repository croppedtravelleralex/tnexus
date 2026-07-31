#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
GW="${GW_BASE:-http://43.156.233.219:8014}"
TN_API="http://127.0.0.1:9000"

echo "==> gateway token from panda"
GW_TOKEN=$(ssh panda 'PASS=$(grep AUTH_BOOTSTRAP_ADMIN_PASSWORD /root/gptimage-gateway-rs/secrets/gateway.env | cut -d= -f2-); curl -fsS -c - -X POST http://127.0.0.1:8014/api/auth/login -H "Content-Type: application/json" -d "{\"username\":\"admin\",\"password\":\"$PASS\"}" -o /dev/null | awk "/gws_session/ {print \$7}"')
echo "gw_token_len=${#GW_TOKEN}"
if [[ ${#GW_TOKEN} -lt 50 ]]; then
  echo "bad gateway token" >&2
  exit 1
fi

echo "==> restart tnexus api/worker (gateway upstream)"
pkill -f './target/debug/tnexus-api' 2>/dev/null || true
pkill -f './target/debug/tnexus-worker' 2>/dev/null || true
sleep 1
set -a
# shellcheck disable=SC1091
source .env
set +a
export GPTIMAGE_BASE="$GW"
export UPSTREAM_API_KEY="$GW_TOKEN"
export UPSTREAM_API_BASE="$GW"
nohup ./target/debug/tnexus-api > /tmp/tnexus-api.log 2>&1 &
nohup env DIRECTOR_MODEL=gpt-5-mini CHATGPT_IMAGE_MODEL=gpt-image-2 ./target/debug/tnexus-worker > /tmp/tnexus-worker.log 2>&1 &
sleep 4
curl -fsS "$TN_API/health"
echo

CJ=/tmp/tnexus-url-chain-cj
rm -f "$CJ"
curl -fsS -c "$CJ" -X POST "$TN_API/api/auth/login" \
  -H 'Content-Type: application/json' \
  -H 'Origin: http://localhost:3010' \
  -d '{"email":"admin","password":"123456"}' > /dev/null

JOB_ID=$(curl -fsS -b "$CJ" -X POST "$TN_API/api/jobs" \
  -H 'Content-Type: application/json' \
  -d '{
    "mode":"director",
    "workflow_path":"full_agent",
    "ps_enabled":false,
    "provider":"chatgpt",
    "director_models":["gpt"],
    "gen_config":{"quality":"auto","width":1024,"height":1024,"count":1,"transparent_bg":false},
    "director_factors":{"x":0,"y":0},
    "ps_factors":{"x":0,"y":0},
    "input_prompt":"a red cube on a white background, product photo, studio lighting"
  }' | python3 -c 'import sys,json; print(json.load(sys.stdin)["job_id"])')
echo "job_id=$JOB_ID"

for i in $(seq 1 120); do
  DETAIL=$(curl -fsS -b "$CJ" "$TN_API/api/jobs/$JOB_ID")
  STATUS=$(python3 -c 'import sys,json; print(json.load(sys.stdin)["status"])' <<<"$DETAIL")
  echo "poll_$i status=$STATUS"
  if [[ "$STATUS" == "done" ]]; then
    PREVIEW=$(python3 -c 'import sys,json; d=json.load(sys.stdin); r=d.get("results") or []; print((r[0] or {}).get("preview_url") or "")' <<<"$DETAIL")
    echo "preview_url=$PREVIEW"
    if [[ -z "$PREVIEW" || "$PREVIEW" != http* ]]; then
      echo "$DETAIL" | head -c 2000
      exit 1
    fi
    BYTES=$(curl -fsS "$PREVIEW" | wc -c)
    echo "preview_bytes=$BYTES"
    PHASE=$(python3 -c 'import sys,json; d=json.load(sys.stdin); print(json.dumps(d.get("phase_timings_ms") or {}, indent=2))' <<<"$DETAIL")
    echo "phase_timings_ms=$PHASE"
    echo "TNEXUS_URL_CHAIN_OK"
    exit 0
  fi
  if [[ "$STATUS" == "failed" ]]; then
    echo "$DETAIL" | python3 -m json.tool | head -80
    tail -40 /tmp/tnexus-worker.log
    exit 1
  fi
  sleep 5
done
tail -40 /tmp/tnexus-worker.log
exit 1
