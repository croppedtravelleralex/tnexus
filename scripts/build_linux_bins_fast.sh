#!/usr/bin/env bash
# 在 rust:1-bookworm 内快速编译 Linux 二进制（glibc 兼容 Panda/Debian）
# 加速：复用 ~/.cargo 缓存 + target 放 WSL ext4；优先 --offline 免拉网
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WSL_ROOT="$ROOT"
CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}"
TARGET_DIR="${TNEXUS_TARGET_DIR:-$HOME/.cache/tnexus-target}"

mkdir -p "$TARGET_DIR" "$CARGO_HOME"

echo ">>> fast Linux build (bookworm)"
echo "    source: $WSL_ROOT"
echo "    target: $TARGET_DIR"
echo "    cargo:  $CARGO_HOME"

build_inner() {
  local offline_flag="$1"
  docker run --rm --network host \
    -v "$WSL_ROOT:/app:ro" \
    -v "$CARGO_HOME:/root/.cargo" \
    -v "$TARGET_DIR:/target" \
    -w /app \
    -e CARGO_TARGET_DIR=/target \
    -e CARGO_NET_RETRY=5 \
    rust:1-bookworm \
    bash -c "
      set -e
      export PATH=\"/usr/local/cargo/bin:\$PATH\"
      cd /app
      test -f rust-toolchain.toml && mv rust-toolchain.toml /tmp/rt.bak 2>/dev/null || true
      cargo build --release $offline_flag -p tnexus-api -p tnexus-worker
    "
}

if build_inner "--offline" 2>/dev/null; then
  echo ">>> built offline (no network)"
else
  echo ">>> offline miss — online build with host network + cargo cache"
  build_inner ""
fi

install -D "$TARGET_DIR/release/tnexus-api" "$WSL_ROOT/target/release/tnexus-api"
install -D "$TARGET_DIR/release/tnexus-worker" "$WSL_ROOT/target/release/tnexus-worker"
echo ">>> installed to $WSL_ROOT/target/release/"
