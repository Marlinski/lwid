# ---- Builder ----
FROM rust:1.85-bookworm AS builder

WORKDIR /build

# Copy manifests and lockfile first for better layer caching
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/

# Build the server binary in release mode
RUN cargo build --release --package lwid-server

# ---- Runtime ----
FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user
RUN groupadd --system lwid \
    && useradd --system --gid lwid --create-home lwid

# Persistent data directory
RUN mkdir -p /data && chown lwid:lwid /data
VOLUME /data

# Copy shell assets
COPY shell/ /shell/

# Copy the compiled binary
COPY --from=builder /build/target/release/lwid-server /usr/local/bin/lwid-server

# Environment
ENV LWID_STORAGE__DATA_DIR=/data
ENV LWID_SERVER__SHELL_DIR=/shell

EXPOSE 8080

USER lwid

ENTRYPOINT ["lwid-server"]
