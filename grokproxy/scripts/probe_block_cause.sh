#!/usr/bin/env bash
# Is accounts.x.ai blocking the TLS fingerprint, or the IP?
#
# The answer picks the Rust HTTP client: an IP block means plain reqwest through
# the residential relay is fine, a fingerprint block means the port needs an
# impersonating client. Testing plain curl from a datacenter IP alone cannot
# tell these apart, so run the same plain curl through the relay the accounts
# themselves egress from.
set -u
UA='Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/138.0.0.0 Safari/537.36'

echo "=== relays listening on this host ==="
ss -lntp 2>/dev/null | grep -E '18[0-9]{3}|relay' | head -5
echo
echo "=== a proxy_url an account actually uses ==="
proxy="$(sqlite3 /opt/grokproxy/data/grokproxy.db \
  "SELECT proxy_url FROM accounts WHERE proxy_url<>'' LIMIT 1;" 2>/dev/null || echo '')"
relay="${GROKPROXY_STICKY_RELAY:-}"
[[ -z "$relay" ]] && relay="$(grep -oP '(?<=^GROKPROXY_STICKY_RELAY=).*' /opt/grokproxy/.env 2>/dev/null || true)"
echo "stored proxy: ${proxy:0:40}...   sticky relay: ${relay:-none}"

# The stored URL points at whatever host grokProxy rewrites to the real relay.
if [[ -n "$proxy" && -n "$relay" ]]; then
  proxy="$(python3 - "$proxy" "$relay" <<'PY'
import sys
from urllib.parse import urlsplit, urlunsplit
url, relay = sys.argv[1], sys.argv[2]
parts = urlsplit(url)
creds = parts.netloc.rsplit('@', 1)[0] if '@' in parts.netloc else ''
netloc = f"{creds}@{relay}" if creds else relay
print(urlunsplit((parts.scheme, netloc, parts.path, parts.query, parts.fragment)))
PY
)"
fi
echo "using proxy: ${proxy:0:32}..."

probe() {
  local label="$1"; shift
  printf '%-34s ' "$label"
  local code
  code="$(curl -s -o /tmp/pb.html -w '%{http_code}' --max-time 40 "$@" \
          -H "User-Agent: $UA" -L https://accounts.x.ai/ 2>/dev/null || echo 000)"
  local why
  why="$(grep -oiE 'blocked due to abusive|just a moment|sign-in|sign-up|challenge' /tmp/pb.html \
        | head -1)"
  echo "HTTP ${code}  ${why:-clean}  ($(wc -c < /tmp/pb.html) bytes)"
}

echo
echo "=== plain curl, same TLS, different egress ==="
probe "direct (datacenter IP)"
[[ -n "$proxy" ]] && probe "through residential relay" --proxy "$proxy"

echo
echo "=== reference: does auth.x.ai care either way? ==="
printf '%-34s ' "auth.x.ai direct"
curl -s -o /dev/null -w 'HTTP %{http_code}\n' --max-time 30 -H "User-Agent: $UA" \
  -X POST -H 'Content-Type: application/x-www-form-urlencoded' \
  --data-urlencode 'client_id=b1a00492-073a-47ea-816f-4c329264a828' \
  --data-urlencode 'scope=openid profile email offline_access grok-cli:access api:access' \
  https://auth.x.ai/oauth2/device/code

echo
echo "=== verdict ==="
echo "relay clean + direct blocked  -> IP reputation; plain reqwest is enough."
echo "both blocked                  -> TLS fingerprint; the port needs impersonation."
