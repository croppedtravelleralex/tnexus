#!/usr/bin/env bash
# Panda 上测试海外代理能否访问 grok.com（meta + chat 前置）
set -euo pipefail

USER="${GROK_PROXY_USER:?}"
PASS="${GROK_PROXY_PASS:?}"
HOST="${GROK_PROXY_HOST:-70.39.164.200}"
PORT="${GROK_PROXY_PORT:-3000}"
UA='Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36'

HTTP_PROXY="http://${USER}:${PASS}@${HOST}:${PORT}"
SOCKS_PROXY="socks5h://${USER}:${PASS}@${HOST}:${PORT}"

echo "=== HTTP ${HOST}:${PORT} ==="
code=$(curl -sS -o /tmp/grok_home.html -w '%{http_code}' --connect-timeout 15 --max-time 25 \
  -x "$HTTP_PROXY" -A "$UA" -H 'Accept: text/html' "https://grok.com/" || echo 000)
echo "status=$code bytes=$(wc -c </tmp/grok_home.html 2>/dev/null || echo 0)"
grep -oE 'grok-site-verification|Just a moment|name="gr[^"]*"' /tmp/grok_home.html 2>/dev/null | head -3 || true

echo "=== SOCKS ${HOST}:${PORT} ==="
code2=$(curl -sS -o /tmp/grok_home2.html -w '%{http_code}' --connect-timeout 15 --max-time 25 \
  -x "$SOCKS_PROXY" -A "$UA" "https://grok.com/" || echo 000)
echo "status=$code2 bytes=$(wc -c </tmp/grok_home2.html 2>/dev/null || echo 0)"
grep -oE 'grok-site-verification|Just a moment' /tmp/grok_home2.html 2>/dev/null | head -2 || true

if [[ "${TRY_30000:-}" == "1" ]]; then
  echo "=== HTTP ${HOST}:30000 ==="
  curl -sS -o /dev/null -w "status=%{http_code}\n" --connect-timeout 15 \
    -x "http://${USER}:${PASS}@${HOST}:30000" -A "$UA" "https://grok.com/" || true
fi
