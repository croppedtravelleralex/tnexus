#!/usr/bin/env bash
# 从 NewAPI :8081 压测 grok-vision-ocr。
# 用法（Panda）：
#   CONCURRENCY=8 TOTAL=40 bash /root/TNexus/scripts/panda_newapi_ocr_bench.sh
set -uo pipefail

NEWAPI="${NEWAPI_BASE:-http://127.0.0.1:8081}"
FIXTURE="${OCR_FIXTURE:-/root/TNexus/tests/grok_golden/ocr_fixture_tnexus.png}"
CONCURRENCY="${CONCURRENCY:-8}"
TOTAL="${TOTAL:-40}"
OCR_MARK="TNEXUS-OCR-OK"

TOKEN=$(docker exec new-api-postgres psql -U newapi -d new-api -t -A -c \
  "SELECT key FROM tokens WHERE name='tnexus-ocr-key' AND deleted_at IS NULL LIMIT 1;" \
  | tr -d '[:space:]')
if [[ -z "$TOKEN" ]]; then
  echo "missing tnexus-ocr-key" >&2
  exit 2
fi
if [[ ! -f "$FIXTURE" ]]; then
  echo "missing fixture $FIXTURE" >&2
  exit 2
fi

B64=$(python3 - "$FIXTURE" <<'PY'
import base64, pathlib, sys
print(base64.b64encode(pathlib.Path(sys.argv[1]).read_bytes()).decode("ascii"))
PY
)

export NEWAPI TOKEN B64 OCR_MARK
python3 - "$CONCURRENCY" "$TOTAL" <<'PY'
import json, os, re, sys, time, urllib.request
from concurrent.futures import ThreadPoolExecutor, as_completed

concurrency = int(sys.argv[1])
total = int(sys.argv[2])
url = os.environ["NEWAPI"].rstrip("/") + "/v1/chat/completions"
token = os.environ["TOKEN"]
b64 = os.environ["B64"]
mark = re.sub(r"\s+", "", os.environ["OCR_MARK"]).lower()
body = json.dumps({
    "model": "grok-vision-ocr",
    "stream": False,
    "messages": [{
        "role": "user",
        "content": [
            {"type": "image_url", "image_url": {"url": f"data:image/png;base64,{b64}"}},
            {"type": "text", "text": "把看到的字写出来，不要解释。"},
        ],
    }],
}).encode()

def one(i):
    req = urllib.request.Request(
        url, data=body, method="POST",
        headers={
            "Authorization": f"Bearer sk-{token}",
            "Content-Type": "application/json",
        },
    )
    t0 = time.perf_counter()
    try:
        with urllib.request.urlopen(req, timeout=90) as resp:
            raw = resp.read().decode("utf-8", "replace")
            code = resp.status
    except urllib.error.HTTPError as e:
        raw = e.read().decode("utf-8", "replace")
        code = e.code
    except Exception as e:
        return i, 0, False, str(e), (time.perf_counter() - t0) * 1000
    ms = (time.perf_counter() - t0) * 1000
    text = ""
    try:
        text = json.loads(raw)["choices"][0]["message"]["content"]
    except Exception:
        text = raw[:200]
    compact = re.sub(r"\s+", "", text).lower()
    ok = code == 200 and mark in compact
    return i, code, ok, text[:80].replace("\n", " "), ms

lat = []
ok = 0
fail = 0
print(f"=== NewAPI OCR bench concurrency={concurrency} total={total} ===", flush=True)
with ThreadPoolExecutor(max_workers=concurrency) as pool:
    futs = [pool.submit(one, i) for i in range(total)]
    for fut in as_completed(futs):
        i, code, success, snippet, ms = fut.result()
        lat.append(ms)
        if success:
            ok += 1
            flag = "PASS"
        else:
            fail += 1
            flag = "FAIL"
        print(f"{flag} #{i:02d} http={code} {ms:.0f}ms {snippet}", flush=True)

lat.sort()
def pct(p):
    if not lat:
        return 0
    idx = min(len(lat) - 1, max(0, int(round((p / 100) * (len(lat) - 1)))))
    return lat[idx]

print()
print(f"ok={ok} fail={fail} success={ok * 100 / total:.1f}%")
print(f"latency_ms min={lat[0]:.0f} p50={pct(50):.0f} p95={pct(95):.0f} max={lat[-1]:.0f}")
sys.exit(0 if fail == 0 else 1)
PY
