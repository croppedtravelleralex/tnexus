# syntax=docker/dockerfile:1
# grokProxy — built by CI, pulled on Panda. Never build this on Panda.
FROM rust:1-bookworm AS builder
WORKDIR /app
# clang and cmake build BoringSSL, which wreq needs: accounts.x.ai answers a
# browser TLS signature with 307 and refuses reqwest's with 403, so the mint
# flow cannot be driven by a plain client.
RUN apt-get update \
    && apt-get install -y --no-install-recommends pkg-config clang cmake \
    && rm -rf /var/lib/apt/lists/*
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release --locked --bin grokproxy

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/grokproxy /usr/local/bin/grokproxy
# SQLite lives on a mounted volume so redeploys never drop the pool.
VOLUME ["/data"]
ENV GROKPROXY_ADDR=0.0.0.0:8110 \
    GROKPROXY_DB=/data/grokproxy.db
EXPOSE 8110
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl -sf http://127.0.0.1:8110/healthz || exit 1
ENTRYPOINT ["/usr/local/bin/grokproxy"]
