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

# busybox:musl is itself FROM scratch + a ~1 MB static binary plus a
# directory of relative symlinks (sh → busybox, grep → busybox, etc.).
# We copy the whole /bin so GitLab Runner's prelude script — which
# invokes grep, mkdir, sed, tar and a handful of other coreutils — has
# what it needs. The ~1 MB binary is the only real disk cost; the
# symlinks share the same inode-equivalent on the image layer.
FROM busybox:musl AS shell

# /etc/ssl/certs/ca-certificates.crt is load-bearing — every assay
# HTTPS call (reqwest, sqlx TLS, WebSockets) verifies certs against
# it. Without this file on FROM scratch every TLS connection fails.
FROM scratch
COPY --from=certs /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/
COPY --from=shell /bin /bin
# assay goes under /usr/local/bin/ so the bare `assay` command resolves
# via the default PATH (=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin).
# Scripts like `assay foo.lua` work without an absolute path.
COPY --from=picked /assay /usr/local/bin/assay
ENTRYPOINT ["/usr/local/bin/assay"]
