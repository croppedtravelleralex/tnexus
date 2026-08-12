#!/usr/bin/env bash
# 验证 a2ecd32 的生图路由修复：
#   1) /v1/chat/completions + gpt-image-2 应走图片路径（此前掉进文本路径 → 413）
#   2) 超长 history 应返回 400 client 错误，而不是 502
#   3) /v1/images/generations 基线仍正常
set -uo pipefail

GW=http://127.0.0.1:8014
K=$(grep -E '^GATEWAY_AUTH_KEY=' /opt/tnexus/.env | cut -d= -f2- | tr -d '\r\n')
TS=$(date +%s)
TMP=$(mktemp -d); trap 'rm -rf "$TMP"' EXIT

hdr=(-H "Authorization: Bearer $K" -H 'Content-Type: application/json')

echo "=== 1) chat/completions + gpt-image-2 (以前走文本路径) ==="
cat >"$TMP/chat.json" <<EOF
{"model":"gpt-image-2","messages":[
  {"role":"system","content":"You are a helpful assistant."},
  {"role":"user","content":"a red cube on white background $TS"}
],"stream":false}
EOF
CODE=$(curl -sS -o "$TMP/chat.out" -w '%{http_code}' --max-time 430 "${hdr[@]}" \
  --data-binary @"$TMP/chat.json" "$GW/v1/chat/completions")
echo "http=$CODE"
python3 - "$TMP/chat.out" <<'PY'
import json,sys
try:
    d=json.load(open(sys.argv[1],encoding="utf-8"))
except Exception as e:
    print("  (non-json)", open(sys.argv[1],encoding="utf-8",errors="replace").read()[:300]); raise SystemExit
if d.get("error"):
    print("  error:", json.dumps(d["error"],ensure_ascii=False)[:300])
    raise SystemExit
msg=(d.get("choices") or [{}])[0].get("message",{})
# image path returns the PNG in `tnexus_image_b64`, not in `content`
b64=str(msg.get("tnexus_image_b64") or "")
content=str(msg.get("content") or "")
print(f"  content_len={len(content)} image_b64_len={len(b64)}")
print("  VERDICT:", "IMAGE PATH OK" if len(b64) > 1000 else "NO IMAGE (still text path?)")
PY

echo
echo "=== 2) 超长 history 应为 400 client，不应是 502 ==="
python3 - "$TMP/long.json" "$TS" <<'PY'
import json,sys
big = "x" * 40000
body = {"model":"gpt-4o","messages":[
    {"role":"user","content": big},
    {"role":"user","content": f"summarize {sys.argv[2]}"}
], "stream": False}
json.dump(body, open(sys.argv[1],"w",encoding="utf-8"))
PY
CODE=$(curl -sS -o "$TMP/long.out" -w '%{http_code}' --max-time 60 "${hdr[@]}" \
  --data-binary @"$TMP/long.json" "$GW/v1/chat/completions")
echo "http=$CODE (期望 400)"
head -c 300 "$TMP/long.out"; echo

echo
echo "=== 3) images/generations 基线 ==="
CODE=$(curl -sS -o /dev/null -w '%{http_code}' --max-time 430 "${hdr[@]}" \
  -d "$(printf '{"model":"gpt-image-2","prompt":"routing check %s","n":1,"size":"256x256","response_format":"b64_json"}' "$TS")" \
  "$GW/v1/images/generations")
echo "http=$CODE (期望 200)"
