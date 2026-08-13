#!/usr/bin/env bash
# Diff account_token TTL between chatgpt2api-local (source of truth) and panda gateway (:8014 consumer)
set -uo pipefail

TMP=$(mktemp -d); trap 'rm -rf "$TMP"' EXIT

UKEY=$(python3 -c "import json;print(json.load(open('/root/gptimage/config.json',encoding='utf-8')).get('auth-key',''))")
GKEY=$(grep -E '^GATEWAY_AUTH_KEY=' /opt/tnexus/.env | cut -d= -f2- | tr -d '\r\n')

curl -sS --max-time 40 -H "Authorization: Bearer $UKEY" http://127.0.0.1:8012/api/accounts -o "$TMP/up.json"
curl -sS --max-time 40 -H "Authorization: Bearer $GKEY" http://127.0.0.1:8014/api/accounts -o "$TMP/gw.json"
echo "upstream_bytes=$(wc -c < "$TMP/up.json") gateway_bytes=$(wc -c < "$TMP/gw.json")"

python3 - "$TMP/up.json" "$TMP/gw.json" <<'PY'
import base64, json, sys, time

def load(p):
    d = json.load(open(p, encoding="utf-8"))
    return d if isinstance(d, list) else (d.get("accounts") or d.get("data") or d.get("items") or [])

def jwt_exp(tok):
    try:
        p = tok.split(".")[1]; p += "=" * (-len(p) % 4)
        return json.loads(base64.urlsafe_b64decode(p)).get("exp")
    except Exception:
        return None

up, gw = load(sys.argv[1]), load(sys.argv[2])
now = int(time.time())

def idx(items):
    m = {}
    for a in items:
        tok = a.get("access_token") or ""
        e = jwt_exp(tok)
        m[a.get("email", "?")] = {
            "ttl_h": None if e is None else round((e - now) / 3600, 1),
            "tok8": tok[-8:],
            "err": (a.get("last_token_refresh_error") or "").strip()[:50],
            "sched": a.get("image_schedulable"),
        }
    return m

U, G = idx(up), idx(gw)
print("upstream_accounts=%d gateway_accounts=%d" % (len(U), len(G)))
print()
print("%-38s %10s %10s %8s  %s" % ("email", "up_ttl_h", "gw_ttl_h", "same_tok", "note"))
stale = same = 0
for email in sorted(set(U) | set(G)):
    u, g = U.get(email), G.get(email)
    if not u:
        print("%-38s %10s %10s %8s  %s" % (email, "-", g["ttl_h"], "-", "MISSING_UPSTREAM")); continue
    if not g:
        print("%-38s %10s %10s %8s  %s" % (email, u["ttl_h"], "-", "-", "MISSING_GATEWAY")); continue
    st = u["tok8"] == g["tok8"]
    note = ""
    if not st:
        note = "STALE_IN_GATEWAY"
        stale += 1
    else:
        same += 1
    if g["ttl_h"] is not None and g["ttl_h"] < 0:
        note += " gw_EXPIRED"
    print("%-38s %10s %10s %8s  %s" % (email, u["ttl_h"], g["ttl_h"], "Y" if st else "N", note))

print()
print("token_identical=%d token_stale_in_gateway=%d" % (same, stale))
PY
