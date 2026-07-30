#!/usr/bin/env bash
set -euo pipefail
# Panda deploy helper — pull image and restart (no build on Panda)
cd /opt/tnexus
docker compose -f deploy/panda/docker-compose.yml pull
docker compose -f deploy/panda/docker-compose.yml up -d
curl -fsS http://127.0.0.1:9000/health | head -c 200
echo
