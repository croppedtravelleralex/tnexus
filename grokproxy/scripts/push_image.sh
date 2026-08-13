#!/usr/bin/env bash
# Push grokproxy tags to GHCR with retry.
#
# CI is the intended builder; this is the fallback used while GitHub Actions is
# blocked. The sha tag is what makes the running image traceable to a commit,
# so it must succeed — latest alone is not enough.
set -u
sha="${1:?usage: push_image.sh <commit-sha>}"
owner="${GHCR_OWNER:-croppedtravelleralex}"

push_with_retry() {
  local ref="$1"
  for attempt in 1 2 3 4 5; do
    if docker push "$ref" 2>&1 | tail -1 | grep -q 'digest:'; then
      echo "pushed $ref"
      return 0
    fi
    echo "retry $attempt for $ref"
    sleep $((attempt * 5))
  done
  echo "FAILED $ref" >&2
  return 1
}

push_with_retry "ghcr.io/${owner}/grokproxy:${sha}"
push_with_retry "ghcr.io/${owner}/grokproxy:latest"
