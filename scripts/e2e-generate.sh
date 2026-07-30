#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
COOKIE=/tmp/tnexus-cookie.txt

curl -s -c "$COOKIE" -b "$COOKIE" -X POST http://localhost:9000/api/auth/login \
  -H "Content-Type: application/json" \
  -d '{"email":"admin","password":"123456"}'
echo

JOB=$(curl -s -c "$COOKIE" -b "$COOKIE" -X POST http://localhost:9000/api/jobs \
  -H "Content-Type: application/json" \
  -d '{"mode":"director","workflow_path":"full_agent","ps_enabled":false,"provider":"chatgpt","director_models":["gpt"],"director_factors":{"x":0.5,"y":0.5},"ps_factors":{"x":0.5,"y":0.5},"input_prompt":"a serene mountain lake at sunset, cinematic","gen_config":{"quality":"auto","width":1024,"height":1024,"transparent_bg":false},"actor_image_counts":{"gpt":1}}')
echo "$JOB"
JOB_ID=$(python3 -c "import json,sys; print(json.load(sys.stdin)['job_id'])" <<< "$JOB")
echo "job_id=$JOB_ID"

for i in $(seq 1 60); do
  sleep 5
  R=$(curl -s -c "$COOKIE" -b "$COOKIE" "http://localhost:9000/api/jobs/$JOB_ID")
  STATUS=$(python3 -c "import json,sys; print(json.load(sys.stdin)['status'])" <<< "$R")
  echo "attempt $i status=$STATUS"
  if [[ "$STATUS" == "done" || "$STATUS" == "failed" ]]; then
    echo "FINAL:"
    python3 -c "import json,sys; d=json.load(sys.stdin); print('status',d.get('status')); print('results',len(d.get('results',[]))); r=d.get('results',[]); print('preview', (r[0].get('preview_url') or '')[:80] if r else 'none')" <<< "$R"
    break
  fi
done
