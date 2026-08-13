#!/usr/bin/env bash
set -u

echo "=== gateway container env (masked) ==="
docker inspect panda-gateway-1 --format '{{range .Config.Env}}{{println .}}{{end}}' \
  | sed -E 's/(KEY|TOKEN|SECRET|PASSWORD)=(.{0,6}).*/\1=\2...(masked)/I'

echo
echo "=== gateway cmd ==="
docker inspect panda-gateway-1 --format 'Entrypoint={{json .Config.Entrypoint}} Cmd={{json .Config.Cmd}}'

echo
echo "=== gateway mounts ==="
docker inspect panda-gateway-1 --format '{{range .Mounts}}{{.Source}} -> {{.Destination}} ({{.Mode}}){{println}}{{end}}'

echo
echo "=== gateway.env candidates ==="
for f in /root/gptimage-gateway-rs/secrets/gateway.env /opt/tnexus/gateway.env /opt/tnexus/.env; do
  if [ -f "$f" ]; then
    echo "--- $f ---"
    sed -E 's/(KEY|TOKEN|SECRET|PASSWORD|URL)=(.{0,20}).*/\1=\2...(trunc)/I' "$f" | head -40
  fi
done

echo
echo "=== who polls /api/accounts/panda-sync (upstream endpoint impl) ==="
docker exec chatgpt2api-local sh -c "grep -rn -B3 -A 30 'panda-sync' /app/api/accounts.py | head -70"

echo
echo "=== gateway logs: account load / sync ==="
docker logs --since 24h panda-gateway-1 2>&1 | grep -iE 'account|sync|pool|refresh' | tail -30
