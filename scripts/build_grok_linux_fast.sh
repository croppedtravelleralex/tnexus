#!/usr/bin/env bash
# 在 rust:1-bookworm 内编译 grok2api-rs（glibc 兼容 Panda）
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WSL_ROOT="$ROOT"
CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}"
TARGET_DIR="${TNEXUS_TARGET_DIR:-$HOME/.cache/tnexus-target}"

mkdir -p "$TARGET_DIR" "$CARGO_HOME"

echo ">>> fast Linux build grok2api-rs (bookworm)"
docker run --rm --network host \
  -v "$WSL_ROOT:/app:ro" \
  -v "$CARGO_HOME:/root/.cargo" \
  -v "$TARGET_DIR:/target" \
  -w /app \
  -e CARGO_TARGET_DIR=/target \
  rust:1-bookworm \
  bash -c '
    set -e
    export PATH="/usr/local/cargo/bin:$PATH"
    cd /app
    test -f rust-toolchain.toml && mv rust-toolchain.toml /tmp/rt.bak 2>/dev/null || true
    cargo build --release -p grok2api-rs
  '

install -D "$TARGET_DIR/release/grok2api-rs" "$WSL_ROOT/target/release/grok2api-rs"
echo ">>> installed to $WSL_ROOT/target/release/grok2api-rs"
