#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET_DIR="${TNEXUS_TARGET_DIR:-$HOME/.cache/tnexus-target}"
CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}"
mkdir -p "$TARGET_DIR" "$CARGO_HOME"
docker run --rm --network host \
  -v "$ROOT:/app:ro" \
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
    cargo build --release --offline -p grok2api-rs
  '
install -D "$TARGET_DIR/release/grok2api-rs" "$ROOT/target/release/grok2api-rs"
echo ">>> installed $(ls -lh "$ROOT/target/release/grok2api-rs")"
