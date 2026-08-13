#!/usr/bin/env bash
set -u

echo "=== account-ops env (masked) ==="
docker inspect panda-account-ops-1 --format '{{range .Config.Env}}{{println .}}{{end}}' \
  | sed -E 's/(KEY|TOKEN|SECRET|PASSWORD)=(.{0,6}).*/\1=\2...(masked)/I'

echo
echo "=== account-ops cmd + mounts ==="
docker inspect panda-account-ops-1 --format 'Entrypoint={{json .Config.Entrypoint}} Cmd={{json .Config.Cmd}}'
docker inspect panda-account-ops-1 --format '{{range .Mounts}}{{.Source}} -> {{.Destination}}{{println}}{{end}}'

echo
echo "=== account-ops logs (last 60) ==="
docker logs --tail 60 panda-account-ops-1 2>&1

echo
echo "=== postgres accounts table freshness ==="
docker exec panda-postgres-1 psql -U tnexus -d tnexus -c "\dt" 2>&1 | head -30
