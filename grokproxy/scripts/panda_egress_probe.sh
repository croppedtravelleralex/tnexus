#!/usr/bin/env bash
# Read-only: can grokproxy actually reach the upstream, directly and via the
# per-account sticky relay that the imported credentials carry?
set -u
target="https://auth.x.ai/oauth2/token"

echo "--- from host, direct ---"
curl -s -o /dev/null -w 'host_direct=%{http_code} t=%{time_total}s\n' --max-time 10 "$target" || echo "host_direct=failed"

echo "--- from host, via 172.20.0.1:18100 (sticky relay) ---"
curl -s -o /dev/null -w 'host_relay=%{http_code} t=%{time_total}s\n' --max-time 10 \
  -x "http://172.20.0.1:18100" "$target" || echo "host_relay=failed"

echo "--- from grokproxy container, direct ---"
docker exec grokproxy sh -c \
  "curl -s -o /dev/null -w 'ctr_direct=%{http_code} t=%{time_total}s\n' --max-time 10 '$target'" \
  2>&1 || echo "ctr_direct=failed"

echo "--- from grokproxy container, via relay ---"
docker exec grokproxy sh -c \
  "curl -s -o /dev/null -w 'ctr_relay=%{http_code} t=%{time_total}s\n' --max-time 10 -x http://172.20.0.1:18100 '$target'" \
  2>&1 || echo "ctr_relay=failed"

echo "--- relay listening? ---"
ss -lntp 2>/dev/null | grep -E ':18100' || echo "nothing on 18100"
