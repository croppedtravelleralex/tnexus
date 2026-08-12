#!/usr/bin/env bash
# 本地构建并推送 GHCR（Actions 分钟用尽时的合规发布链路）
# 用法：bash scripts/build_push_ghcr.sh [tnexus|grok|gateway|all]
# 前置：docker login ghcr.io -u croppedtravelleralex（PAT 需 write:packages）
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OWNER="${GHCR_OWNER:-croppedtravelleralex}"
TAG="${IMAGE_TAG:-latest}"
SHA="$(git -C "$ROOT" rev-parse --short HEAD)"
API_BASE="${NEXT_PUBLIC_API_BASE:-https://tnexus.relai.asia}"
RUNTIME_IMAGE="${TNEXUS_RUNTIME_IMAGE:-tnexus-runtime:bookworm}"

# debian 镜像源慢时 apt 层能跑十几分钟，而 repack 每次都会重来一遍。
# 做成一次性本地基础镜像，后续 repack 只剩一层 COPY。
ensure_runtime_image() {
  if docker image inspect "$RUNTIME_IMAGE" >/dev/null 2>&1; then
    return
  fi
  echo ">>> building $RUNTIME_IMAGE (one-off)"
  local empty_ctx
  empty_ctx="$(mktemp -d)"
  docker build --network host -f "$ROOT/Dockerfile.runtime" -t "$RUNTIME_IMAGE" "$empty_ctx"
  rmdir "$empty_ctx"
}

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

build_gateway() {
  echo ">>> repack tnexus-gateway from prebuilt artifact ($TAG + $SHA)"
  local bin="$ROOT/target/release/gptimage-gateway-rs"
  # 在 bookworm 容器里编译（复用增量 target），而不是在镜像层里冷编译整个依赖树。
  if [[ "${FORCE_REBUILD:-0}" == "1" ]] || [[ ! -f "$bin" ]]; then
    bash "$ROOT/scripts/build_linux_bins_fast.sh" gateway:gptimage-gateway-rs
  elif ! docker run --rm -v "$bin:/bin/gw:ro" --entrypoint true debian:bookworm-slim /bin/gw 2>/dev/null; then
    echo ">>> existing binary incompatible with bookworm, rebuilding"
    bash "$ROOT/scripts/build_linux_bins_fast.sh" gateway:gptimage-gateway-rs
  else
    echo ">>> skip cargo (binary exists and runs on bookworm; FORCE_REBUILD=1 to override)"
  fi
  ensure_runtime_image
  local stage="$ROOT/dist/docker-gateway"
  rm -rf "$stage"
  mkdir -p "$stage"
  cp "$bin" "$stage/"
  docker build --network host -f "$ROOT/Dockerfile.gateway.repack" \
    --build-arg "RUNTIME_IMAGE=$RUNTIME_IMAGE" \
    -t "ghcr.io/$OWNER/tnexus-gateway:$TAG" \
    -t "ghcr.io/$OWNER/tnexus-gateway:$SHA" \
    "$stage"
  docker push "ghcr.io/$OWNER/tnexus-gateway:$TAG"
  docker push "ghcr.io/$OWNER/tnexus-gateway:$SHA"
}

what="${1:-all}"
case "$what" in
  tnexus) build_tnexus ;;
  grok) build_grok ;;
  gateway) build_gateway ;;
  all) build_tnexus; build_grok; build_gateway ;;
  *) echo "usage: $0 [tnexus|grok|gateway|all]" >&2; exit 2 ;;
esac

echo ">>> done. Panda: ssh panda 'cd /root/TNexus && bash deploy/panda/deploy.sh && bash deploy/panda/grok-bootstrap.sh'"
