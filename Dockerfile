# syntax=docker/dockerfile:1

FROM rust:1.94-bookworm AS builder
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY migrations ./migrations
RUN cargo build --release -p tnexus-api -p tnexus-worker

FROM node:22-bookworm AS web
WORKDIR /web
COPY web/package*.json ./
RUN npm ci
COPY web ./
ARG NEXT_PUBLIC_API_BASE=
ENV NEXT_PUBLIC_API_BASE=$NEXT_PUBLIC_API_BASE
RUN npm run build

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=builder /app/target/release/tnexus-api /usr/local/bin/tnexus-api
COPY --from=builder /app/target/release/tnexus-worker /usr/local/bin/tnexus-worker
COPY --from=web /web/out ./web/out
COPY migrations ./migrations
ENV GATEWAY_STATIC_DIR=/app/web/out
EXPOSE 9000
CMD ["tnexus-api"]
