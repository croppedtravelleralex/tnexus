#!/usr/bin/env bash
# Patch Panda nginx for sub2api.closeapi.top: long-lived /v1/images/* with buffering on.
# Large b64_json responses (~3MB) can stall with proxy_buffering off + default 30s locations.
#
# Usage (on Panda):
#   bash /root/TNexus/deploy/panda/patch_sub2api_image_timeout.sh apply
#   bash /root/TNexus/deploy/panda/patch_sub2api_image_timeout.sh status
#   bash /root/TNexus/deploy/panda/patch_sub2api_image_timeout.sh rollback
set -euo pipefail

CONF="${SUB2API_NGINX_CONF:-/etc/nginx/sites-enabled/sub2api.closeapi.top.conf}"
MARKER="# TNEXUS_IMAGE_PROXY_BLOCK"
BACKUP="${CONF}.bak.tnexus-image"

image_block() {
  cat <<'EOF'
    # TNEXUS_IMAGE_PROXY_BLOCK — long timeouts + buffering for multi-MB b64 JSON
    location ^~ /v1/images/ {
        proxy_pass http://127.0.0.1:8081;
        proxy_http_version 1.1;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
        proxy_connect_timeout 60s;
        proxy_send_timeout 600s;
        proxy_read_timeout 600s;
        send_timeout 600s;
        proxy_buffering on;
        proxy_buffers 64 256k;
        proxy_busy_buffers_size 512k;
        proxy_max_temp_file_size 0;
        client_max_body_size 32m;
        proxy_request_buffering on;
    }
EOF
}

status() {
  if [[ ! -f "$CONF" ]]; then
    echo "missing: $CONF"
    exit 1
  fi
  if grep -q "$MARKER" "$CONF"; then
    echo "image block: present"
    grep -A2 "$MARKER" "$CONF" | head -5
  else
    echo "image block: absent"
  fi
  nginx -t 2>&1 || true
}

apply() {
  if [[ ! -f "$CONF" ]]; then
    echo "missing: $CONF"
    exit 1
  fi
  if grep -q "$MARKER" "$CONF"; then
    echo "already patched"
    status
    return 0
  fi
  cp -a "$CONF" "$BACKUP"
  python3 - <<PY
from pathlib import Path
conf = Path("$CONF")
text = conf.read_text()
block = """$(image_block)"""
# Insert before first "location /" inside server block (after server_name line cluster)
needle = "    location / {"
if needle not in text:
    raise SystemExit("could not find '    location / {' in $CONF")
text = text.replace(needle, block + "\n" + needle, 1)
conf.write_text(text)
print("patched", conf)
PY
  nginx -t
  systemctl reload nginx
  echo "nginx reloaded"
  status
}

rollback() {
  if [[ -f "$BACKUP" ]]; then
    cp -a "$BACKUP" "$CONF"
    nginx -t
    systemctl reload nginx
    echo "restored from $BACKUP"
  else
    echo "no backup at $BACKUP"
    exit 1
  fi
}

case "${1:-status}" in
  apply) apply ;;
  status) status ;;
  rollback) rollback ;;
  *) echo "usage: $0 {apply|status|rollback}"; exit 1 ;;
esac
