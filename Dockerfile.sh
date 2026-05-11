# assay container with /bin/sh — published as ghcr.io/developerinlondon/assay:<version>-sh.
#
# Identical contents to the main `Dockerfile`, plus a static busybox at
# /bin/sh so the image works in environments that require a shell to
# launch the binary — notably GitLab CI's docker executor, which wraps
# every script: line in `sh -c "..."` and crashes before the entrypoint
# ever runs if no shell is present.
#
# K8s consumers that already launch assay with `command: ["/assay", ...]`
# should keep using the plain `:<version>` tag — smaller image, smaller
# CVE surface, no shell available for accidental injection.

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

# Alpine is only a vendored cert source; alpine itself never ships in
# the final image.
FROM alpine:3 AS certs
RUN apk add --no-cache ca-certificates

# busybox:musl is itself FROM scratch + a single ~1 MB static binary.
# We pull `/bin/busybox` out of it and rename to `/bin/sh` in the final
# image — that's all GitLab CI's `sh -c` wrapper needs.
FROM busybox:musl AS shell

# /etc/ssl/certs/ca-certificates.crt is load-bearing — every assay
# HTTPS call (reqwest, sqlx TLS, WebSockets) verifies certs against
# it. Without this file on FROM scratch every TLS connection fails.
FROM scratch
COPY --from=certs /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/
COPY --from=shell /bin/busybox /bin/sh
COPY --from=picked /assay /assay
ENTRYPOINT ["/assay"]
