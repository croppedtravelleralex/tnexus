#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OWNER="${GHCR_OWNER:-croppedtravelleralex}"
SHA="$(git -C "$ROOT" rev-parse --short HEAD)"
stage="$ROOT/dist/docker-grok"
rm -rf "$stage"
mkdir -p "$stage"
cp "$ROOT/target/release/grok2api-rs" "$stage/"
cp "$ROOT/crates/grok-signer/assets/grok_sign_standalone.js" "$stage/"
docker build --network host -f "$ROOT/Dockerfile.grok.repack" \
  -t "ghcr.io/$OWNER/grok2api-rs:latest" \
  -t "ghcr.io/$OWNER/grok2api-rs:$SHA" \
  "$stage"
docker push "ghcr.io/$OWNER/grok2api-rs:latest"
docker push "ghcr.io/$OWNER/grok2api-rs:$SHA"
echo ">>> grok push done"
