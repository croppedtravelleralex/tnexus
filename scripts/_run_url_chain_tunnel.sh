#!/usr/bin/env bash
set -euo pipefail
export NO_PROXY=127.0.0.1,localhost
export no_proxy=127.0.0.1,localhost
ROOT="/mnt/d/SelfMadeTool/TNexus"
cd "$ROOT"
GW="http://127.0.0.1:18014"
TN_API="http://127.0.0.1:9000"

GW_TOKEN=$(ssh panda 'PASS=$(grep AUTH_BOOTSTRAP_ADMIN_PASSWORD /root/gptimage-gateway-rs/secrets/gateway.env | cut -d= -f2-); curl -fsS -c - -X POST http://127.0.0.1:8014/api/auth/login -H "Content-Type: application/json" -d "{\"username\":\"admin\",\"password\":\"$PASS\"}" -o /dev/null | awk "/gws_session/ {print \$7}"')
echo "gw_token_len=${#GW_TOKEN}"

echo "==> probe image via tunnel"
curl -sS --noproxy '*' -X POST "$GW/v1/images/generations" \
  -H "Authorization: Bearer $GW_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"model":"gpt-image-2","prompt":"red cube on white","n":1,"size":"1024x1024","response_format":"url"}' | head -c 500
echo

echo "==> restart api/worker"
pkill -f './target/debug/tnexus-api' 2>/dev/null || true
pkill -f './target/debug/tnexus-worker' 2>/dev/null || true
sleep 1
set -a
source .env
set +a
export GPTIMAGE_BASE="$GW"
export UPSTREAM_API_KEY="$GW_TOKEN"
export UPSTREAM_API_BASE="$GW"
export NO_PROXY=127.0.0.1,localhost
export no_proxy=127.0.0.1,localhost
nohup ./target/debug/tnexus-api > /tmp/tnexus-api.log 2>&1 &
nohup env DIRECTOR_MODEL=gpt-5-mini CHATGPT_IMAGE_MODEL=gpt-image-2 ./target/debug/tnexus-worker > /tmp/tnexus-worker.log 2>&1 &
sleep 4
curl -sS --noproxy '*' "$TN_API/health"
echo

CJ=/tmp/tnexus-url-chain-cj
curl -sS --noproxy '*' -c "$CJ" -X POST "$TN_API/api/auth/login" \
  -H 'Content-Type: application/json' -H 'Origin: http://localhost:3010' \
  -d '{"email":"admin","password":"123456"}' > /dev/null

JOB_ID=$(curl -sS --noproxy '*' -b "$CJ" -X POST "$TN_API/api/jobs" \
  -H 'Content-Type: application/json' \
  -d '{"mode":"director","workflow_path":"full_agent","ps_enabled":false,"provider":"chatgpt","director_models":["gpt"],"gen_config":{"quality":"auto","width":1024,"height":1024,"count":1,"transparent_bg":false},"director_factors":{"x":0,"y":0},"ps_factors":{"x":0,"y":0},"input_prompt":"a red cube on a white background, product photo, studio lighting"}' \
  | python3 -c 'import sys,json; print(json.load(sys.stdin)["job_id"])')
echo "job_id=$JOB_ID"

for i in $(seq 1 120); do
  DETAIL=$(curl -sS --noproxy '*' -b "$CJ" "$TN_API/api/jobs/$JOB_ID")
  STATUS=$(python3 -c 'import sys,json; print(json.load(sys.stdin)["status"])' <<<"$DETAIL")
  echo "poll_$i status=$STATUS"
  if [[ "$STATUS" == "done" ]]; then
    PREVIEW=$(python3 -c 'import sys,json; d=json.load(sys.stdin); r=d.get("results") or []; print((r[0] or {}).get("preview_url") or "")' <<<"$DETAIL")
    echo "preview_url=$PREVIEW"
    BYTES=$(curl -sS --noproxy '*' "$PREVIEW" | wc -c)
    echo "preview_bytes=$BYTES"
    python3 -c 'import sys,json; d=json.load(sys.stdin); print("phase_timings_ms", json.dumps(d.get("phase_timings_ms") or {}))' <<<"$DETAIL"
    echo "TNEXUS_URL_CHAIN_OK"
    exit 0
  fi
  if [[ "$STATUS" == "failed" ]]; then
    echo "$DETAIL" | python3 -m json.tool | head -40
    tail -30 /tmp/tnexus-worker.log
    exit 1
  fi
  sleep 5
done
tail -30 /tmp/tnexus-worker.log
exit 1
