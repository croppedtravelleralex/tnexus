#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

python3 scripts/repair_migrations.py

pkill -f './target/debug/tnexus-api' 2>/dev/null || true
pkill -f './target/debug/tnexus-worker' 2>/dev/null || true
pkill -f 'account_ops_face.py' 2>/dev/null || true
sleep 1
set -a
source .env
set +a

export PYTHONPATH="${GPTIMAGE_ROOT:-}:$(pwd)/helper"
if [[ -f helper/.venv/bin/activate ]]; then
  # shellcheck disable=SC1091
  source helper/.venv/bin/activate
fi
nohup python3 helper/account_ops_face.py > /tmp/tnexus-account-ops.log 2>&1 &

nohup ./target/debug/tnexus-api > /tmp/tnexus-api.log 2>&1 &
nohup ./target/debug/tnexus-worker > /tmp/tnexus-worker.log 2>&1 &
sleep 3
curl -sS http://127.0.0.1:9000/health
echo
curl -sS http://127.0.0.1:9011/health 2>/dev/null || echo "account_ops not up"
echo
curl -sS -X POST http://127.0.0.1:9000/api/auth/login \
  -H 'Content-Type: application/json' \
  -H 'Origin: http://localhost:3010' \
  -d '{"email":"admin","password":"123456"}'
echo
