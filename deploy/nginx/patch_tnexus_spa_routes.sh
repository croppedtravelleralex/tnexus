#!/usr/bin/env bash
set -euo pipefail
CONF=/etc/nginx/sites-available/tnexus.relai.asia.conf
if grep -q 'extensionless console paths' "$CONF"; then
  echo already patched
  exit 0
fi
python3 <<'PY'
from pathlib import Path
p = Path("/etc/nginx/sites-available/tnexus.relai.asia.conf")
t = p.read_text()
needle = "    # TNexus API + static UI"
block = """    # Next.js static export: extensionless console paths → *.html
    location ~ ^/(accounts|ops|logs|chat|image-manager|settings|studio|history|login|register)$ {
        return 302 /$1.html;
    }

    # TNexus API + static UI"""
if needle not in t:
    raise SystemExit("needle not found")
p.write_text(t.replace(needle, block, 1))
print("patched")
PY
nginx -t && systemctl reload nginx
echo nginx reloaded
