#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET_DIR="$HOME/.cache/tnexus-target-docker2"
CARGO_HOME="$HOME/.cargo"
rm -rf "$TARGET_DIR"
mkdir -p "$TARGET_DIR"
echo ">>> offline bookworm build (registry from $CARGO_HOME)"
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
echo ">>> ldd check in bookworm:"
docker run --rm -v "$ROOT/target/release/grok2api-rs:/bin/grok:ro" --entrypoint ldd rust:1-bookworm /bin/grok | head -3
echo ">>> OK"
