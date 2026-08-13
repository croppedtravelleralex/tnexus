#!/usr/bin/env bash
# 仅 repack + push（不跑 cargo）。二进制须已存在于 target/release/
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OWNER="${GHCR_OWNER:-croppedtravelleralex}"
TAG="${IMAGE_TAG:-latest}"
SHA="$(git -C "$ROOT" rev-parse --short HEAD)"

for bin in tnexus-api tnexus-worker grok2api-rs; do
  if [[ ! -f "$ROOT/target/release/$bin" ]]; then
    echo "missing $ROOT/target/release/$bin — run cargo build first" >&2
    exit 1
  fi
done
if [[ ! -d "$ROOT/web/out" ]]; then
  echo "missing web/out — run npm run build in web/" >&2
  exit 1
fi

echo ">>> repack tnexus ($TAG + $SHA)"
stage="$ROOT/dist/docker"
rm -rf "$stage"
mkdir -p "$stage/web-out" "$stage/migrations"
cp "$ROOT/target/release/tnexus-api" "$ROOT/target/release/tnexus-worker" "$stage/"
cp -a "$ROOT/web/out/." "$stage/web-out/"
cp -a "$ROOT/migrations/." "$stage/migrations/"
docker build -f "$ROOT/Dockerfile.repack" \
  -t "ghcr.io/$OWNER/tnexus:$TAG" \
  -t "ghcr.io/$OWNER/tnexus:$SHA" \
  "$stage"
docker push "ghcr.io/$OWNER/tnexus:$TAG"
docker push "ghcr.io/$OWNER/tnexus:$SHA"

echo ">>> repack grok2api-rs ($TAG + $SHA)"
stage="$ROOT/dist/docker-grok"
rm -rf "$stage"
mkdir -p "$stage"
cp "$ROOT/target/release/grok2api-rs" "$stage/"
cp "$ROOT/crates/grok-signer/assets/grok_sign_standalone.js" "$stage/"
docker build --network host -f "$ROOT/Dockerfile.grok.repack" \
  -t "ghcr.io/$OWNER/grok2api-rs:$TAG" \
  -t "ghcr.io/$OWNER/grok2api-rs:$SHA" \
  "$stage"
docker push "ghcr.io/$OWNER/grok2api-rs:$TAG"
docker push "ghcr.io/$OWNER/grok2api-rs:$SHA"

echo ">>> done (repack only, no cargo)"
