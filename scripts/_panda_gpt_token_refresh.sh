#!/usr/bin/env bash
# Refresh expired GPT access_tokens via chatgpt2api-local live HTTP API (safe: runs in app process).
#   bash _panda_gpt_token_refresh.sh --dry-run
#   bash _panda_gpt_token_refresh.sh --apply [--min-ttl 86400]
set -uo pipefail

MODE=dry-run
MIN_TTL=86400
while [ $# -gt 0 ]; do
  case "$1" in
    --dry-run) MODE=dry-run ;;
    --apply)   MODE=apply ;;
    --min-ttl) MIN_TTL="$2"; shift ;;
    *) echo "unknown arg $1" >&2; exit 2 ;;
  esac
  shift
done

BASE=http://127.0.0.1:8012
KEY=$(python3 -c "import json;print(json.load(open('/root/gptimage/config.json',encoding='utf-8')).get('auth-key',''))")
if [ -z "$KEY" ]; then echo "FATAL: no auth-key in config.json" >&2; exit 1; fi

TMP=$(mktemp -d); trap 'rm -rf "$TMP"' EXIT

echo "=== proactive-refresh loop status ==="
curl -sS --max-time 20 -H "Authorization: Bearer $KEY" "$BASE/api/accounts/proactive-refresh/status" \
  | python3 -c "import json,sys
try: print(json.dumps(json.load(sys.stdin),ensure_ascii=False,indent=2)[:1500])
except Exception as e: print('parse-fail',e)"

echo
echo "=== fetching account list ==="
curl -sS --max-time 40 -H "Authorization: Bearer $KEY" "$BASE/api/accounts" -o "$TMP/acc.json"
echo "bytes=$(wc -c < "$TMP/acc.json")"

MODE="$MODE" MIN_TTL="$MIN_TTL" python3 - "$TMP/acc.json" "$TMP/targets.json" <<'PY'
import base64, json, os, sys, time

src, dst = sys.argv[1], sys.argv[2]
mode = os.environ["MODE"]
min_ttl = int(os.environ["MIN_TTL"])

raw = json.load(open(src, encoding="utf-8"))
items = raw if isinstance(raw, list) else (raw.get("accounts") or raw.get("data") or raw.get("items") or [])

def jwt_exp(tok):
    try:
        p = tok.split(".")[1]; p += "=" * (-len(p) % 4)
        return json.loads(base64.urlsafe_b64decode(p)).get("exp")
    except Exception:
        return None

now = int(time.time())
expired, soon, healthy = [], [], []
for a in items:
    tok = a.get("access_token") or ""
    email = a.get("email", "?")
    err = (a.get("last_token_refresh_error") or "").strip()
    exp = jwt_exp(tok)
    rec = {"email": email, "token": tok, "ttl_h": None if exp is None else round((exp - now) / 3600, 1), "err": err[:60]}
    if exp is None or exp < now:
        expired.append(rec)
    elif exp - now < min_ttl:
        soon.append(rec)
    else:
        healthy.append(rec)

print(f"total={len(items)} EXPIRED={len(expired)} expiring_soon(<{min_ttl}s)={len(soon)} healthy={len(healthy)}")
print("\n-- expired --")
for r in sorted(expired, key=lambda x: x["email"]):
    print("  %-38s ttl_h=%-8s err=%s" % (r["email"], r["ttl_h"], r["err"]))
if soon:
    print("-- expiring soon --")
    for r in sorted(soon, key=lambda x: x["email"]):
        print("  %-38s ttl_h=%-8s" % (r["email"], r["ttl_h"]))

targets = expired + soon
json.dump({"tokens": [r["token"] for r in targets],
           "emails": [r["email"] for r in targets]}, open(dst, "w", encoding="utf-8"))
print(f"\nTARGETS={len(targets)} mode={mode}")
PY

N=$(python3 -c "import json;print(len(json.load(open('$TMP/targets.json'))['tokens']))")
if [ "$N" -eq 0 ]; then echo "nothing to refresh"; exit 0; fi

if [ "$MODE" != "apply" ]; then
  echo "DRY-RUN: would POST /api/accounts/refresh with $N tokens (no changes made)"
  exit 0
fi

echo
echo "=== APPLY: POST /api/accounts/refresh (n=$N) ==="
python3 -c "
import json
d=json.load(open('$TMP/targets.json'))
json.dump({'access_tokens':d['tokens']},open('$TMP/body.json','w'))
"
curl -sS --max-time 60 -X POST -H "Authorization: Bearer $KEY" -H 'Content-Type: application/json' \
  --data-binary @"$TMP/body.json" "$BASE/api/accounts/refresh" -o "$TMP/resp.json"
cat "$TMP/resp.json"; echo

PID=$(python3 -c "import json;print(json.load(open('$TMP/resp.json')).get('progress_id',''))" 2>/dev/null)
if [ -z "$PID" ]; then echo "no progress_id returned; aborting poll"; exit 1; fi

echo
echo "=== polling progress $PID ==="
for i in $(seq 1 90); do
  sleep 10
  curl -sS --max-time 20 -H "Authorization: Bearer $KEY" "$BASE/api/accounts/refresh/progress/$PID" -o "$TMP/p.json" 2>/dev/null
  DONE=$(python3 - "$TMP/p.json" <<'PY'
import json,sys
try: d=json.load(open(sys.argv[1],encoding='utf-8'))
except Exception: print('?|?|?|0'); raise SystemExit
done=d.get('done') or d.get('completed') or d.get('processed') or 0
total=d.get('total') or 0
fin=d.get('finished') or d.get('finished_at') or d.get('status')=='finished' or (total and done>=total)
print(f"{done}|{total}|{str(d.get('error') or '')[:80]}|{1 if fin else 0}")
PY
)
  echo "  [$i] $DONE"
  case "$DONE" in *"|1") break ;; esac
done

echo
echo "=== final progress ==="
curl -sS --max-time 20 -H "Authorization: Bearer $KEY" "$BASE/api/accounts/refresh/progress/$PID" \
  | python3 -c "import json,sys;print(json.dumps(json.load(sys.stdin),ensure_ascii=False,indent=2)[:3000])"
