#!/usr/bin/env bash
# Read-only: is the grokproxy image published and pullable from this host?
# Compares against a known-good image so an auth problem is not mistaken for
# a build that has not finished.
set -u
owner="${GHCR_OWNER:-croppedtravelleralex}"

for img in grok2api-rs grokproxy; do
  ref="ghcr.io/${owner}/${img}:latest"
  if out="$(docker manifest inspect "$ref" 2>&1)"; then
    digest="$(printf '%s' "$out" | grep -m1 -o 'sha256:[0-9a-f]\{12\}')"
    echo "${img}: READY ${digest}"
  else
    echo "${img}: NOT_READY -- $(printf '%s' "$out" | tail -1 | cut -c1-100)"
  fi
done

echo "--- ghcr credentials on this host ---"
if grep -q 'ghcr.io' /root/.docker/config.json 2>/dev/null; then
  echo "ghcr entry present in /root/.docker/config.json"
else
  echo "no ghcr entry (public pulls only)"
fi
