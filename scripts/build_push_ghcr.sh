#!/usr/bin/env bash
# 本地构建并推送 GHCR（Actions 分钟用尽时的合规发布链路）
# 用法：bash scripts/build_push_ghcr.sh [tnexus|grok|all]
# 前置：docker login ghcr.io -u croppedtravelleralex（PAT 需 write:packages）
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OWNER="${GHCR_OWNER:-croppedtravelleralex}"
TAG="${IMAGE_TAG:-latest}"
SHA="$(git -C "$ROOT" rev-parse --short HEAD)"
API_BASE="${NEXT_PUBLIC_API_BASE:-https://tnexus.relai.asia}"

build_tnexus() {
  echo ">>> repack tnexus from prebuilt artifacts ($TAG + $SHA)"
  local need_build=0
  if [[ "${FORCE_REBUILD:-0}" == "1" ]]; then
    need_build=1
  elif [[ ! -f "$ROOT/target/release/tnexus-api" ]]; then
    need_build=1
  elif [[ "$ROOT/crates/tnexus-api/src/routes/grok_gateway.rs" -nt "$ROOT/target/release/tnexus-api" ]]; then
    need_build=1
  fi
  if [[ "$need_build" == "1" ]]; then
    bash "$ROOT/scripts/build_linux_bins_fast.sh"
  else
    echo ">>> skip cargo (binary newer than grok_gateway.rs; FORCE_REBUILD=1 to override)"
  fi
  if [[ ! -d "$ROOT/web/out" ]]; then
    echo "missing $ROOT/web/out — run: cd web && NEXT_PUBLIC_API_BASE=$API_BASE npm run build" >&2
    exit 1
  fi
  local stage="$ROOT/dist/docker"
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
}

build_grok() {
  echo ">>> build grok2api-rs ($TAG + $SHA) in rust:1-bookworm"
  if [[ ! -f "$ROOT/target/release/grok2api-rs" ]] || [[ "${FORCE_REBUILD:-0}" == "1" ]]; then
    bash "$ROOT/scripts/build_grok_bookworm.sh"
  else
    # 验证二进制能在 bookworm 运行（防止 WSL 直编 glibc 污染）
    if ! docker run --rm -v "$ROOT/target/release/grok2api-rs:/bin/grok:ro" --entrypoint true rust:1-bookworm /bin/grok 2>/dev/null; then
      echo ">>> existing binary incompatible with bookworm, rebuilding"
      bash "$ROOT/scripts/build_grok_bookworm.sh"
    else
      echo ">>> skip cargo (grok2api-rs exists; FORCE_REBUILD=1 to override)"
    fi
  fi
  local stage="$ROOT/dist/docker-grok"
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
}

what="${1:-all}"
case "$what" in
  tnexus) build_tnexus ;;
  grok) build_grok ;;
  all) build_tnexus; build_grok ;;
  *) echo "usage: $0 [tnexus|grok|all]" >&2; exit 2 ;;
esac

echo ">>> done. Panda: ssh panda 'cd /root/TNexus && bash deploy/panda/deploy.sh && bash deploy/panda/grok-bootstrap.sh'"
