#!/usr/bin/env bash
# How many stored web SSO cookies are still good, and can one of them mint?
#
# validate_sso is a single GET, so scanning is cheap. The distribution is worth
# knowing on its own: it says whether the web pool is a usable mint source or
# just a pile of expired cookies.
set -u
set -a; . /opt/grokproxy/.env; set +a
db=/opt/grokproxy/data/grokproxy.db
relay="${GROKPROXY_STICKY_RELAY:-127.0.0.1:18100}"
limit="${1:-15}"

mapfile -t rows < <(sqlite3 -separator '|' "$db" \
  "SELECT email, sso_token FROM accounts
    WHERE provider='web' AND sso_token<>''
    ORDER BY updated_at DESC LIMIT ${limit};")

echo "trying ${#rows[@]} web accounts"
live=""
for row in "${rows[@]}"; do
  email="${row%%|*}"
  sso="${row#*|}"
  session="mint$(printf '%s' "$email" | md5sum | cut -c1-10)"
  proxy="http://${session}:sticky@${relay}"

  out="$(python3 - "$email" "$sso" "$proxy" "$GROKPROXY_ADMIN_KEY" <<'PY'
import json, sys, urllib.request, urllib.error
email, sso, proxy, key = sys.argv[1:5]
body = json.dumps({"email": email, "sso_token": sso, "proxy_url": proxy}).encode()
req = urllib.request.Request(
    "http://127.0.0.1:8110/api/v1/mint", data=body,
    headers={"Content-Type": "application/json", "Authorization": f"Bearer {key}"})
try:
    with urllib.request.urlopen(req, timeout=300) as r:
        print(json.dumps(json.loads(r.read().decode())))
except urllib.error.HTTPError as e:
    print(e.read().decode())
except Exception as e:
    print(json.dumps({"ok": False, "error": f"{type(e).__name__}: {e}"}))
PY
)"

  verdict="$(printf '%s' "$out" | python3 -c 'import json,sys
d = json.loads(sys.stdin.read() or "{}")
if d.get("ok"):
    print("MINTED " + json.dumps(d.get("account")))
else:
    err = str(d.get("error", ""))
    for needle, label in (("sso rejected", "sso expired"),
                          ("blocked by the edge", "egress blocked"),
                          ("principal id", "consent page shape changed"),
                          ("device code", "device code refused"),
                          ("Access denied", "approval not accepted")):
        if needle in err:
            print(label); break
    else:
        print(err[:110])')"
  printf '  %-34s %s\n' "${email:0:34}" "$verdict"
  [[ "$verdict" == MINTED* ]] && { live="$email"; break; }
done

echo
if [[ -n "$live" ]]; then
  echo "=== the account that minted ==="
  sqlite3 -header -column "$db" \
    "SELECT email, provider, health, length(refresh_token) AS refresh_len,
            datetime(expires_at,'unixepoch') AS expires
       FROM accounts WHERE email='${live}' AND provider='build';"
else
  echo "no stored web SSO is still valid; a fresh registration is needed to"
  echo "exercise the full mint path."
fi
