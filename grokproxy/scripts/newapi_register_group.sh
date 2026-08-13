#!/usr/bin/env bash
# Make a group usable in newapi.
#
# A channel can name any group it likes, but unless that group also appears in
# GroupRatio (billing) and UserUsableGroups (access), every request against it
# is refused with "无权访问 X 分组". Creating the channel is only half the wiring.
set -euo pipefail

GROUP="${1:?usage: newapi_register_group.sh <group> [ratio] [description]}"
RATIO="${2:-0.1}"
DESC="${3:-}"

psql() { docker exec new-api-postgres psql -U newapi -d new-api -A -t "$@"; }

# Patch the two JSON blobs in place rather than rewriting them, so unrelated
# groups configured through the UI survive.
psql -v ON_ERROR_STOP=1 -q -c "
  UPDATE options
     SET value = (value::jsonb || jsonb_build_object('${GROUP}', ${RATIO}::numeric))::text
   WHERE key = 'GroupRatio';
  UPDATE options
     SET value = (value::jsonb || jsonb_build_object('${GROUP}', '${DESC}'))::text
   WHERE key = 'UserUsableGroups';"

echo "registered group ${GROUP} (ratio ${RATIO})"
psql -F'|' -c "SELECT key, value::jsonb ->> '${GROUP}' FROM options
                WHERE key IN ('GroupRatio','UserUsableGroups');"

# newapi caches options in memory at boot; without a restart the new group stays
# invisible no matter what the table says.
echo ">>> restarting new-api so it reloads the option cache"
docker restart new-api >/dev/null
for _ in $(seq 1 30); do
  code="$(docker exec new-api sh -c 'wget -q -T 2 -O /dev/null -S http://127.0.0.1:3000/api/status 2>&1 | grep -c "200 OK"' || echo 0)"
  [[ "$code" != "0" ]] && { echo "new-api back up"; exit 0; }
  sleep 2
done
echo "new-api did not report healthy within 60s" >&2
exit 1
