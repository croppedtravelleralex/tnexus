#!/usr/bin/env bash
# Feasibility gate for porting the mint flow to Rust.
#
# The Python path drives these endpoints through curl_cffi impersonating
# Chrome. If they also answer a plain TLS client, reqwest is enough; if they
# only answer a browser fingerprint, the Rust port needs an impersonating
# client (rquest) and that decision has to be made before any code is written.
set -u
UA='Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/138.0.0.0 Safari/537.36'
CLIENT_ID='b1a00492-073a-47ea-816f-4c329264a828'
SCOPE='openid profile email offline_access grok-cli:access api:access'

echo "=== plain curl TLS fingerprint ==="
curl --version | head -1

echo
echo "=== 1. POST auth.x.ai/oauth2/device/code (no cookie needed) ==="
code="$(curl -s -o /tmp/dc.json -w '%{http_code}' --max-time 30 \
  -H "User-Agent: $UA" -H 'Content-Type: application/x-www-form-urlencoded' \
  --data-urlencode "client_id=${CLIENT_ID}" \
  --data-urlencode "scope=${SCOPE}" \
  https://auth.x.ai/oauth2/device/code)"
echo "HTTP $code"
head -c 400 /tmp/dc.json; echo

echo
echo "=== 2. GET accounts.x.ai/ (the SSO validation step) ==="
sso="$(docker exec grokproxy sh -c 'true' 2>/dev/null; \
      sqlite3 /opt/grokproxy/data/grokproxy.db \
      "SELECT sso_token FROM accounts WHERE provider='web' AND sso_token<>'' LIMIT 1;" 2>/dev/null || echo '')"
if [[ -z "$sso" ]]; then
  echo "(no web sso token available; testing without a cookie)"
  curl -s -o /tmp/acc.html -w 'HTTP %{http_code}  final=%{url_effective}\n' --max-time 30 \
    -H "User-Agent: $UA" -L https://accounts.x.ai/
else
  echo "(using a stored web sso token)"
  curl -s -o /tmp/acc.html -w 'HTTP %{http_code}  final=%{url_effective}\n' --max-time 30 \
    -H "User-Agent: $UA" -H "Cookie: sso=${sso}; sso-rw=${sso}" -L https://accounts.x.ai/
fi
grep -oiE 'blocked due to abusive|just a moment|cf-challenge|sign-in|sign-up' /tmp/acc.html \
  | sort -u | head -5
echo "(body $(wc -c < /tmp/acc.html) bytes)"

echo
echo "=== 3. GET accounts.x.ai/oauth2/device/consent (the consent page) ==="
curl -s -o /tmp/con.html -w 'HTTP %{http_code}\n' --max-time 30 \
  -H "User-Agent: $UA" "https://accounts.x.ai/oauth2/device/consent?user_code=TEST-TEST"
grep -oiE 'blocked due to abusive|just a moment|cf-challenge' /tmp/con.html | sort -u | head -3
echo "(body $(wc -c < /tmp/con.html) bytes)"

echo
echo "=== 4. POST auth.x.ai/oauth2/token (expect a clean OAuth error, not a block) ==="
curl -s -o /tmp/tok.json -w 'HTTP %{http_code}\n' --max-time 30 \
  -H "User-Agent: $UA" -H 'Content-Type: application/x-www-form-urlencoded' \
  --data-urlencode 'grant_type=urn:ietf:params:oauth:grant-type:device_code' \
  --data-urlencode "client_id=${CLIENT_ID}" \
  --data-urlencode 'device_code=not-a-real-code' \
  https://auth.x.ai/oauth2/token
head -c 300 /tmp/tok.json; echo

echo
echo "=== verdict ==="
echo "A clean JSON answer above (even an OAuth error) means the endpoint does not"
echo "gate on TLS fingerprint. An HTML challenge or 403 block means it does."
