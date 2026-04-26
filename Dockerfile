# assay container image — published as `ghcr.io/developerinlondon/assay`.
# Pure Lua runtime; for the engine server image see `Dockerfile.assay-engine`.
#
# Post-0.13.0 the root-level `src/` and `stdlib/` moved under
# `crates/assay/`, so `COPY crates/ crates/` alone covers every source
# file the source-mode build needs.
#
# Two build modes selected via the `BUILD_MODE` build arg:
#
#   BUILD_MODE=artifact (default)
#     Consumes a pre-built linux-musl binary from the build context.
#     The binary must be present at `assay-linux-x86_64`. Used by the
#     release pipeline after the `binaries` GH Actions job downloads
#     the matching cross-compiled artifact.
#
#     For local use, produce the binary first:
#       cargo build --release --target x86_64-unknown-linux-musl \
#         -p assay-lua --bin assay
#       cp target/x86_64-unknown-linux-musl/release/assay \
#         ./assay-linux-x86_64
#       docker build -t assay .
#
#   BUILD_MODE=source
#     Cross-compiles inside Docker. Slow (~5 min). Useful only when no
#     local Rust toolchain is available.
#
#       docker build --build-arg BUILD_MODE=source -t assay .
#
# `RUST_VERSION` defaults to the pin in `.mise.toml` (the single source
# of truth for the workspace toolchain). The release workflow extracts
# it from .mise.toml and passes it as a build arg.

ARG BUILD_MODE=artifact
ARG RUST_VERSION=1.95

# ──────────────────────────────────────────────────────────────────────
# artifact branch — copy pre-built musl binary from build context
# ──────────────────────────────────────────────────────────────────────
FROM scratch AS artifact
COPY assay-linux-x86_64 /assay

# ──────────────────────────────────────────────────────────────────────
# source branch — full cross-compile (rare, local-dev only)
# ──────────────────────────────────────────────────────────────────────
# `perl` is included for symmetry with Dockerfile.assay-engine — assay-lua
# itself doesn't currently pull openssl-src, but adding a future dep
# that does shouldn't silently start failing.
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

# Normalise the source-builder output to a stage exposing /assay,
# matching the artifact stage's layout.
FROM source-builder AS source

# ──────────────────────────────────────────────────────────────────────
# Picker — Docker allows ARG interpolation in `FROM` but not in
# `COPY --from=…`, so this small stage resolves BUILD_MODE; the runtime
# stage then does a fixed `COPY --from=picked`.
# ──────────────────────────────────────────────────────────────────────
FROM ${BUILD_MODE} AS picked

# ──────────────────────────────────────────────────────────────────────
# CA certificate bundle — alpine is used only as a vendored cert source.
# Files COPYed into FROM scratch; alpine itself never ships.
# ──────────────────────────────────────────────────────────────────────
#
# History: commit 5c43c83 (Feb 2026) flipped this to alpine:3.21 to
# accommodate one downstream Deployment that used `command: ["/bin/sh",
# "-c", …]` wrappers for env-var setup. That wrapper has since been
# removed and assay's stdlib covers the sed/awk use cases that once
# required a shell. `tests/dockerfile.rs` guards against the regression.
#
# The CA bundle COPY is load-bearing: any HTTPS call made by assay
# (reqwest, sqlx TLS, WebSockets) verifies the server cert against
# roots read from /etc/ssl/certs/ca-certificates.crt. Without this
# file on scratch, every TLS connection fails immediately.
FROM alpine:3 AS certs
RUN apk add --no-cache ca-certificates

# ──────────────────────────────────────────────────────────────────────
# Runtime — FROM scratch so the published image is just the assay binary
# plus a CA bundle, nothing else (~12 MB instead of ~25 MB with alpine,
# and no transitive busybox/apk/musl CVE surface).
# ──────────────────────────────────────────────────────────────────────
FROM scratch
COPY --from=certs /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/
COPY --from=picked /assay /assay
ENTRYPOINT ["/assay"]
