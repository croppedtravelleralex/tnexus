#!/usr/bin/env bash
set -euo pipefail
ROOT="/mnt/d/SelfMadeTool/TNexus"
TARGET="/home/lenovo/.cache/tnexus-target"
mkdir -p "$TARGET"

# 离线构建：复用 WSL ~/.cargo 已下载的 crate（避免 Docker 内拉 crates.io）
docker run --rm --network host \
  -v "$ROOT:/app:ro" \
  -v "$HOME/.cargo:/root/.cargo" \
  -v "$TARGET:/target" \
  -w /app \
  -e CARGO_TARGET_DIR=/target \
  rust:1-bookworm \
  bash -c '
    export PATH="/usr/local/cargo/bin:$PATH"
    test -f rust-toolchain.toml && mv rust-toolchain.toml /tmp/rt.bak 2>/dev/null || true
    cargo build --release --offline -p tnexus-api -p tnexus-worker
  '
