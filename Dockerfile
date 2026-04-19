# Builder
FROM rust:1.92-slim AS builder
RUN apt-get update && apt-get install -y musl-tools cmake make g++ protobuf-compiler && rm -rf /var/lib/apt/lists/*
RUN rustup target add x86_64-unknown-linux-musl
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src/ src/
COPY stdlib/ stdlib/
COPY crates/ crates/
RUN cargo build --release --target x86_64-unknown-linux-musl

# Runtime — FROM scratch so the published image is the assay binary
# plus a CA bundle, nothing else (~10 MB instead of ~25 MB with
# alpine, and no transitive busybox/apk/musl CVE surface).
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
FROM scratch
COPY --from=builder /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/
COPY --from=builder /app/target/x86_64-unknown-linux-musl/release/assay /assay
ENTRYPOINT ["/assay"]
