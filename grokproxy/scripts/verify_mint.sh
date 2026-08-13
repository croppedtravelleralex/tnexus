#!/usr/bin/env bash
# Does the in-process mint actually produce a Build credential?
#
# Uses a stored web SSO and the sticky egress that account already has, since
# the consent host judges the IP as well as the TLS signature.
set -u
set -a; . /opt/grokproxy/.env; set +a
db=/opt/grokproxy/data/grokproxy.db

read -r email sso proxy <<<"$(sqlite3 -separator ' ' "$db" \
  "SELECT email, sso_token, proxy_url FROM accounts
    WHERE provider='web' AND sso_token<>'' AND health='active'
    ORDER BY updated_at DESC LIMIT 1;")"

if [[ -z "${email:-}" ]]; then
  echo "no web account with an sso token to test with" >&2
  exit 1
fi
# Stored proxies point at whatever address grokProxy rewrites to the relay.
relay="${GROKPROXY_STICKY_RELAY:-}"
if [[ -n "$proxy" && -n "$relay" ]]; then
  proxy="$(python3 - "$proxy" "$relay" <<'PY'
import sys
from urllib.parse import urlsplit, urlunsplit
url, relay = sys.argv[1], sys.argv[2]
p = urlsplit(url)
creds = p.netloc.rsplit('@', 1)[0] if '@' in p.netloc else ''
print(urlunsplit((p.scheme, f"{creds}@{relay}" if creds else relay, p.path, p.query, p.fragment)))
PY
)"
fi

echo "minting for ${email}  egress=${proxy:0:34}..."
start=$(date +%s)
python3 - "$email" "$sso" "$proxy" "$GROKPROXY_ADMIN_KEY" <<'PY' > /tmp/mint.json
import json, sys, urllib.request
email, sso, proxy, key = sys.argv[1:5]
body = json.dumps({"email": email, "sso_token": sso, "proxy_url": proxy}).encode()
req = urllib.request.Request(
    "http://127.0.0.1:8110/api/v1/mint", data=body,
    headers={"Content-Type": "application/json", "Authorization": f"Bearer {key}"})
try:
    with urllib.request.urlopen(req, timeout=300) as r:
        print(r.read().decode())
except urllib.error.HTTPError as e:
    print(e.read().decode())
except Exception as e:
    print(json.dumps({"ok": False, "error": f"{type(e).__name__}: {e}"}))
PY
echo "took $(( $(date +%s) - start ))s"
python3 -m json.tool < /tmp/mint.json

echo
echo "=== did a Build account appear for this email? ==="
sqlite3 -header -column "$db" \
  "SELECT email, provider, health, length(refresh_token) AS refresh_len,
          datetime(expires_at,'unixepoch') AS expires
     FROM accounts WHERE email='${email}' AND provider='build';"

echo
echo "=== grokproxy log for this mint ==="
docker logs --since 6m grokproxy 2>&1 | grep -iE 'mint|consent|device|sso' | tail -12
