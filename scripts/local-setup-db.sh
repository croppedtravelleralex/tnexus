#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

if [[ ! -f .env ]]; then
  cp .env.example .env
fi

sudo service postgresql start 2>/dev/null || true
sudo service redis-server start 2>/dev/null || true

sudo -u postgres psql -c "CREATE USER tnexus WITH PASSWORD 'tnexus';" 2>/dev/null || true
sudo -u postgres psql -c "CREATE DATABASE tnexus OWNER tnexus;" 2>/dev/null || true

echo "DB ready"
redis-cli ping
sudo -u postgres psql -d tnexus -c "SELECT 1"
