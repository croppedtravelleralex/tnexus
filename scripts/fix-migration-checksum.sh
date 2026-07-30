#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
set -a && source .env && set +a
VER="${1:-2}"
FILE=$(ls migrations/${VER}_*.sql | head -1)
HASH=$(openssl dgst -sha384 -binary "$FILE" | xxd -p -c 256)
psql "$DATABASE_URL" -c "UPDATE _sqlx_migrations SET checksum = decode('$HASH', 'hex') WHERE version = $VER;"
echo "Fixed checksum for migration $VER ($FILE)"
