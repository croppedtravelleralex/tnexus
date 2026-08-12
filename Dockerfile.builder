# 本地编译用基础镜像：rust:1-bookworm + 原生依赖（btls-sys/BoringSSL 需要 cmake+clang）。
# 单独成像是为了让 apt 层被缓存——否则每次 `docker run rust:1-bookworm` 都要重装一遍。
# glibc 与 Panda 的 debian:bookworm-slim 对齐，勿改基础镜像。
FROM rust:1-bookworm
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        libclang-dev clang cmake pkg-config \
    && rm -rf /var/lib/apt/lists/*
