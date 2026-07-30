from pathlib import Path

script = r"""#!/usr/bin/env bash
set -euo pipefail
source "$HOME/.cargo/env" 2>/dev/null || true
export PATH="$HOME/.cargo/bin:$PATH"
GW=http://127.0.0.1:18014
TN_API=http://127.0.0.1:9000
ROOT=/mnt/d/SelfMadeTool/TNexus
cd "$ROOT"
GW_TOKEN=$(ssh panda 'PASS=$(grep AUTH_BOOTSTRAP_ADMIN_PASSWORD /root/gptimage-gateway-rs/secrets/gateway.env | cut -d= -f2-); curl -fsS -c - -X POST http://127.0.0.1:8014/api/auth/login -H "Content-Type: application/json" -d "{\"username\":\"admin\",\"password\":\"$PASS\"}" -o /dev/null | awk "/gws_session/ {print \$7}"')
echo gw_token_len=${#GW_TOKEN}
pkill -f target/debug/tnexus-api 2>/dev/null || true
pkill -f target/debug/tnexus-worker 2>/dev/null || true
cargo build -p tnexus-api -p tnexus-worker -q
nohup env GPTIMAGE_BASE=$GW UPSTREAM_API_KEY=$GW_TOKEN DATABASE_URL=postgres://tnexus:tnexus@localhost:5432/tnexus REDIS_URL=redis://127.0.0.1:6379 JWT_SECRET=change-me-to-a-long-random-secret-at-least-32-chars LISTEN_ADDR=0.0.0.0:9000 CORS_ORIGINS=http://localhost:3000 ./target/debug/tnexus-api > /tmp/tnexus-api.log 2>&1 &
nohup env GPTIMAGE_BASE=$GW UPSTREAM_API_KEY=$GW_TOKEN DATABASE_URL=postgres://tnexus:tnexus@localhost:5432/tnexus REDIS_URL=redis://127.0.0.1:6379 DIRECTOR_MODEL=gpt-5-mini CHATGPT_IMAGE_MODEL=gpt-image-2 ./target/debug/tnexus-worker > /tmp/tnexus-worker.log 2>&1 &
sleep 5
curl -fsS $TN_API/health
TN_TOKEN=$(curl -fsS -X POST $TN_API/api/auth/login -H "Content-Type: application/json" -d '{"email":"demo","password":"demo1234"}' | python3 -c 'import sys,json; print(json.load(sys.stdin)["token"])')
JOB_ID=$(curl -fsS -X POST $TN_API/api/jobs -H "Authorization: Bearer $TN_TOKEN" -H "Content-Type: application/json" -d '{"mode":"director","workflow_path":"full_agent","ps_enabled":false,"provider":"chatgpt","director_models":["gpt"],"gen_config":{"quality":"auto","width":1024,"height":1024,"count":1,"transparent_bg":false},"director_factors":{"x":0,"y":0},"ps_factors":{"x":0,"y":0},"input_prompt":"green triangle on white"}' | python3 -c 'import sys,json; print(json.load(sys.stdin)["job_id"])')
echo job_id=$JOB_ID
for i in $(seq 1 120); do
  DETAIL=$(curl -fsS -H "Authorization: Bearer $TN_TOKEN" $TN_API/api/jobs/$JOB_ID)
  STATUS=$(python3 -c 'import sys,json; print(json.load(sys.stdin)["status"])' <<<"$DETAIL")
  echo poll_$i $STATUS
  if [ "$STATUS" = done ]; then
    PREVIEW=$(python3 -c 'import sys,json; d=json.load(sys.stdin); r=d.get("results") or []; print((r[0] or {}).get("preview_url") or "")' <<<"$DETAIL")
    echo preview_url=$PREVIEW
    curl -fsS "$PREVIEW" | wc -c
    echo TNEXUS_URL_CHAIN_OK
    exit 0
  fi
  if [ "$STATUS" = failed ]; then echo "$DETAIL"; tail -20 /tmp/tnexus-worker.log; exit 1; fi
  sleep 5
done
exit 1
"""

out = Path(__file__).with_name("test_url_chain.sh")
out.write_text(script, newline="\n")
print(f"wrote {out} ({len(script)} bytes)")
