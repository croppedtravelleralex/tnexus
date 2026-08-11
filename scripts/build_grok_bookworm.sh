#!/usr/bin/env bash
# 在 rust:1-bookworm 内编译（glibc 兼容 Panda），使用独立 target 目录避免被宿主机 glibc 污染
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET_DIR="${TNEXUS_TARGET_DIR:-$HOME/.cache/tnexus-target-docker}"
CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}"
mkdir -p "$TARGET_DIR" "$CARGO_HOME"
echo ">>> bookworm build grok2api-rs -> $TARGET_DIR"
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
    cargo build --release -p grok2api-rs
  '
install -D "$TARGET_DIR/release/grok2api-rs" "$ROOT/target/release/grok2api-rs"
echo ">>> installed $(ls -lh "$ROOT/target/release/grok2api-rs")"
# 验证链接到 bookworm glibc
docker run --rm rust:1-bookworm ldd /dev/null 2>/dev/null || true
echo ">>> verify binary runs in bookworm:"
docker run --rm -v "$ROOT/target/release/grok2api-rs:/bin/grok2api-rs:ro" rust:1-bookworm /bin/grok2api-rs --help 2>&1 | head -3 || echo "(no --help, try ldd)"
docker run --rm -v "$ROOT/target/release/grok2api-rs:/bin/grok2api-rs:ro" --entrypoint ldd rust:1-bookworm /bin/grok2api-rs 2>&1 | head -5
