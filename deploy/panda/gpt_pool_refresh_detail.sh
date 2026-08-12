#!/usr/bin/env bash
# 逐账号打印 gptimage-gateway 池内 access_token 过期与刷新错误
set -uo pipefail

K=$(grep -E '^GATEWAY_AUTH_KEY=' /opt/tnexus/.env | cut -d= -f2- | tr -d '\r\n')
GW=http://127.0.0.1:8014
TMP=$(mktemp)
trap 'rm -f "$TMP"' EXIT

curl -sS --max-time 20 -H "Authorization: Bearer $K" "$GW/api/accounts" -o "$TMP" 2>/dev/null

python3 - "$TMP" <<'PY'
import base64
import json
import sys
import time

d = json.load(open(sys.argv[1], encoding="utf-8"))
items = d if isinstance(d, list) else (d.get("accounts") or d.get("data") or d.get("items") or [])


def jwt_exp(tok):
    try:
        p = tok.split(".")[1]
        p += "=" * (-len(p) % 4)
        return json.loads(base64.urlsafe_b64decode(p)).get("exp")
    except Exception:
        return None


now = int(time.time())
expired = live = unknown = 0
rows = []
for a in items:
    email = a.get("email", "?")
    exp = jwt_exp(a.get("access_token") or "")
    if exp is None:
        state, ttl = "unknown", None
        unknown += 1
    elif exp < now:
        state, ttl = "EXPIRED", exp - now
        expired += 1
    else:
        state, ttl = "live", exp - now
        live += 1
    rows.append({
        "state": state,
        "email": email,
        "ttl": ttl,
        "err": (a.get("last_token_refresh_error") or "")[:80],
        "err_at": a.get("last_token_refresh_error_at"),
        "last_at": a.get("last_token_refresh_at"),
        "has_rt": bool(a.get("refresh_token")),
        "streak": a.get("image_fail_streak"),
        "sched": a.get("image_schedulable"),
    })

rows.sort(key=lambda r: (r["state"] != "EXPIRED", r["email"]))
hdr = "%-8s %-38s %8s %3s %5s %6s  %s" % (
    "state", "email", "ttl_h", "rt", "fail", "sched", "last_token_refresh_error")
print(hdr)
for r in rows:
    tt = "%8.1f" % (r["ttl"] / 3600) if r["ttl"] is not None else "       -"
    print("%-8s %-38s %s %3s %5s %6s  %s" % (
        r["state"], r["email"], tt,
        "Y" if r["has_rt"] else "N",
        r["streak"], r["sched"], r["err"]))

print()
print("TOTAL=%d live=%d EXPIRED=%d unknown=%d" % (len(items), live, expired, unknown))
no_rt = [a.get("email") for a in items if not a.get("refresh_token")]
print("no_refresh_token=%d %s" % (len(no_rt), no_rt[:10]))
errs = {}
for a in items:
    e = (a.get("last_token_refresh_error") or "").strip()
    if e:
        errs[e[:90]] = errs.get(e[:90], 0) + 1
print("refresh_error_kinds:")
for e, n in sorted(errs.items(), key=lambda x: -x[1]):
    print("  %3d  %s" % (n, e))
PY
