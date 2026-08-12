#!/usr/bin/env bash
# 在 bookworm 容器内编译 Linux 二进制（glibc 与 Panda 对齐）
# 加速：复用 ~/.cargo 与放在 ext4 上的持久 target；优先 --offline 免拉网
#
# 用法：
#   scripts/build_linux_bins_fast.sh                          # tnexus-api + tnexus-worker
#   scripts/build_linux_bins_fast.sh gateway:gptimage-gateway-rs
#   参数形如 <cargo package>:<产出二进制名>
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WSL_ROOT="$ROOT"
CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}"
# 放 ext4，不要放 /mnt/d：9p 文件系统会让增量编译慢一个数量级
TARGET_DIR="${TNEXUS_TARGET_DIR:-$HOME/.cache/tnexus-target}"
BUILDER_IMAGE="${TNEXUS_BUILDER_IMAGE:-tnexus-builder:bookworm}"

SPECS=("$@")
if [[ ${#SPECS[@]} -eq 0 ]]; then
  SPECS=(tnexus-api:tnexus-api tnexus-worker:tnexus-worker)
fi

PKG_FLAGS=()
BINS=()
for spec in "${SPECS[@]}"; do
  PKG_FLAGS+=(-p "${spec%%:*}")
  BINS+=("${spec##*:}")
done

mkdir -p "$TARGET_DIR" "$CARGO_HOME"

# 构建器镜像带 cmake/clang（btls-sys 需要）；apt 层缓存在镜像里，避免每次 docker run 重装。
if ! docker image inspect "$BUILDER_IMAGE" >/dev/null 2>&1; then
  echo ">>> building $BUILDER_IMAGE (one-off)"
  # 空 context：Dockerfile.builder 没有 COPY，传仓库根目录只会白白拷贝几百 MB
  empty_ctx="$(mktemp -d)"
  docker build --network host -f "$ROOT/Dockerfile.builder" -t "$BUILDER_IMAGE" "$empty_ctx"
  rmdir "$empty_ctx"
fi

echo ">>> fast Linux build (bookworm)"
echo "    packages: ${SPECS[*]}"
echo "    source:   $WSL_ROOT"
echo "    target:   $TARGET_DIR"
echo "    cargo:    $CARGO_HOME"

build_inner() {
  local offline_flag="$1"
  docker run --rm --network host \
    -v "$WSL_ROOT:/app:ro" \
    -v "$CARGO_HOME:/root/.cargo" \
    -v "$TARGET_DIR:/target" \
    -w /app \
    -e CARGO_TARGET_DIR=/target \
    -e CARGO_NET_RETRY=5 \
    "$BUILDER_IMAGE" \
    bash -c "
      set -e
      export PATH=\"/usr/local/cargo/bin:\$PATH\"
      # 源码是只读挂载，改不动 rust-toolchain.toml；用 RUSTUP_TOOLCHAIN 覆盖它。
      # 必须直接读安装目录：在 /app 下跑 'rustup toolchain list' 也会触发 toml 解析，
      # 进而联网重下 1.97（网络受限时会永久挂住）。
      RUSTUP_TOOLCHAIN=\"\$(ls /usr/local/rustup/toolchains | head -1)\"
      export RUSTUP_TOOLCHAIN
      echo \">>> toolchain: \$RUSTUP_TOOLCHAIN\"
      cd /app
      cargo build --release $offline_flag ${PKG_FLAGS[*]}
    "
}

if build_inner "--offline" 2>/dev/null; then
  echo ">>> built offline (no network)"
else
  echo ">>> offline miss — online build with host network + cargo cache"
  build_inner ""
fi

for bin in "${BINS[@]}"; do
  install -D "$TARGET_DIR/release/$bin" "$WSL_ROOT/target/release/$bin"
  echo ">>> installed $WSL_ROOT/target/release/$bin"
done
