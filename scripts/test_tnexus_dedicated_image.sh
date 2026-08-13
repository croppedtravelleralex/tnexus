#!/usr/bin/env bash
# Test TNexus dedicated channel (ch115) via NewAPI :8081
set -euo pipefail

TOKEN=$(docker exec new-api-postgres psql -U newapi -d new-api -tAc \
  "SELECT key FROM tokens WHERE name='tnexus-test-key' AND deleted_at IS NULL AND status=1 LIMIT 1" | tr -d '[:space:]')

if [[ -z "$TOKEN" ]]; then
  echo "ERROR: tnexus-test-key not found" >&2
  exit 1
fi

echo "=== TNexus 专用渠道 ch115 生图测试 ==="
echo "token: ${TOKEN:0:8}... (len=${#TOKEN})"
echo "route: NewAPI :8081 → group=tnexus → ch115 → gateway :8014"
echo ""

START=$(date +%s)
HTTP=$(curl -sS -o /tmp/tnexus_dedicated_resp.json -w "%{http_code}" \
  -X POST http://127.0.0.1:8081/v1/images/generations \
  -H "Authorization: Bearer ${TOKEN}" \
  -H "Content-Type: application/json" \
  -d '{"model":"gpt-image-2","prompt":"a golden retriever puppy in autumn leaves, warm sunlight, photorealistic","n":1,"size":"1024x1024","response_format":"url"}' \
  --max-time 300)
END=$(date +%s)
ELAPSED=$((END - START))

echo "HTTP: ${HTTP}  耗时: ${ELAPSED}s"
echo ""

python3 <<'PY'
import json
from pathlib import Path

raw = Path("/tmp/tnexus_dedicated_resp.json").read_text(encoding="utf-8")
try:
    o = json.loads(raw)
except json.JSONDecodeError:
    print(raw[:800])
    raise SystemExit(1)

if o.get("error"):
    print("error:", o["error"])
    raise SystemExit(1)

data = o.get("data") or []
print("success:", bool(data))
if data:
    url = data[0].get("url", "")
    print("url:", url[:160] + ("..." if len(url) > 160 else ""))
usage = o.get("usage")
if usage:
    print("usage:", usage)
pipe = o.get("_tnexus_pipeline") or {}
if pipe:
    print("account:", pipe.get("account_email"))
    timings = pipe.get("timings_ms") or {}
    print("gateway_wall_ms:", timings.get("gateway_wall_ms"))
    print("quota:", pipe.get("quota_before"), "→", pipe.get("quota_after"))
PY

echo ""
echo "=== 最近 ch115 日志 ==="
docker exec new-api-postgres psql -U newapi -d new-api -c \
  "SELECT id, to_char(to_timestamp(created_at) AT TIME ZONE 'Asia/Shanghai','MM-DD HH24:MI:SS') as t, use_time, type, left(prompt,60) as prompt FROM logs WHERE channel_id=115 ORDER BY id DESC LIMIT 3;"
