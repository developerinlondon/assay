# Builder
FROM rust:1.92-slim AS builder
RUN apt-get update && apt-get install -y musl-tools cmake make g++ && rm -rf /var/lib/apt/lists/*
RUN rustup target add x86_64-unknown-linux-musl
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src/ src/
RUN cargo build --release --target x86_64-unknown-linux-musl

# Runtime
FROM alpine:3.21
COPY --from=builder /app/target/x86_64-unknown-linux-musl/release/assay /usr/local/bin/assay
ENTRYPOINT ["assay"]
