#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

if [[ ! -f .env ]]; then
  cp .env.example .env
fi

export $(grep -v '^#' .env | xargs)

sudo service postgresql start 2>/dev/null || true
sudo service redis-server start 2>/dev/null || true

echo "Starting tnexus-worker ..."
exec ./target/release/tnexus-worker
