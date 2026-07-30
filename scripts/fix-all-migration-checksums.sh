#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
set -a && source .env && set +a

for f in migrations/*.sql; do
  base=$(basename "$f")
  ver=$(echo "$base" | cut -d_ -f1 | sed 's/^0*//')
  hash=$(openssl dgst -sha384 -binary "$f" | xxd -p -c 256)
  psql "$DATABASE_URL" -c "UPDATE _sqlx_migrations SET checksum = decode('$hash', 'hex') WHERE version = $ver;"
  echo "fixed migration $ver"
done
