#!/usr/bin/env bash
# 本地一键启动（WSL）
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

# 修复 Windows CRLF
sed -i 's/\r$//' .env.example 2>/dev/null || true
[[ -f .env ]] && sed -i 's/\r$//' .env || cp .env.example .env

sudo service postgresql start 2>/dev/null || true
sudo service redis-server start 2>/dev/null || true

sudo -u postgres psql -c "CREATE USER tnexus WITH PASSWORD 'tnexus';" 2>/dev/null || true
sudo -u postgres psql -c "CREATE DATABASE tnexus OWNER tnexus;" 2>/dev/null || true

if [[ ! -x ./target/release/tnexus-api ]]; then
  echo "Building Rust binaries (first run may take several minutes)..."
  cargo build --release -p tnexus-api -p tnexus-worker
fi

set -a
source .env
set +a

echo "API:    http://localhost:9000/health"
echo "Web:    http://localhost:3000 (run: cd web && npm run dev)"
echo ""
echo "Starting API + Worker in foreground (Ctrl+C to stop)..."

./target/release/tnexus-api &
API_PID=$!
./target/release/tnexus-worker &
WORKER_PID=$!
trap 'kill $API_PID $WORKER_PID 2>/dev/null' EXIT
wait
