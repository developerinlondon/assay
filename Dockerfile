# assay container — published as ghcr.io/developerinlondon/assay.
# Pure Lua runtime; for the engine server image see Dockerfile.assay-engine.
#
# BUILD_MODE=artifact (default) consumes a pre-built linux-musl binary from
# the build context (file: assay-linux-x86_64). Used by the release pipeline
# after the binaries job uploads/downloads.
#
# BUILD_MODE=source cross-compiles inside Docker (~5 min). Useful only when
# no local Rust toolchain is available:
#   docker build --build-arg BUILD_MODE=source -t assay .
#
# RUST_VERSION must match .mise.toml.

ARG BUILD_MODE=artifact
ARG RUST_VERSION=1.95

FROM scratch AS artifact
COPY assay-linux-x86_64 /assay

# `perl` is included for symmetry with Dockerfile.assay-engine — assay-lua
# itself doesn't currently pull openssl-src, but a future dep that does
# shouldn't silently start failing.
FROM rust:${RUST_VERSION}-slim AS source-builder
RUN apt-get update && apt-get install -y \
      musl-tools cmake make g++ protobuf-compiler perl \
    && rm -rf /var/lib/apt/lists/*
RUN rustup target add x86_64-unknown-linux-musl
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
RUN cargo build --release --target x86_64-unknown-linux-musl -p assay-lua --bin assay \
    && cp /app/target/x86_64-unknown-linux-musl/release/assay /assay

FROM source-builder AS source

# Docker doesn't expand ARG inside `COPY --from=…`; the picker stage
# resolves BUILD_MODE in `FROM` (which does support ARG).
FROM ${BUILD_MODE} AS picked

# Alpine is only a vendored cert source; alpine itself never ships.
# A previous flip to alpine:3.21 (commit 5c43c83, Feb 2026) was
# reverted once the downstream `command: ["/bin/sh", "-c", …]`
# wrapper was removed; tests/dockerfile.rs guards against the
# regression.
FROM alpine:3 AS certs
RUN apk add --no-cache ca-certificates

# /etc/ssl/certs/ca-certificates.crt is load-bearing — every assay
# HTTPS call (reqwest, sqlx TLS, WebSockets) verifies certs against
# it. Without this file on FROM scratch every TLS connection fails.
FROM scratch
COPY --from=certs /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/
COPY --from=picked /assay /assay
ENTRYPOINT ["/assay"]
