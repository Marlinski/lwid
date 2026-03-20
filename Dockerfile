# ---- Builder ----
FROM rust:1.85-bookworm AS builder

WORKDIR /build

# Copy manifests and lockfile first for better layer caching
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/

# Accept git hash for version embedding
ARG GIT_HASH=unknown
ENV GIT_HASH=${GIT_HASH}

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

# Data directory (mount JuiceFS or host volume at /storage)
RUN mkdir -p /storage && chown lwid:lwid /storage

# Copy shell assets (readable by lwid user)
COPY --chown=lwid:lwid shell/ /shell/

# Copy the compiled binary
COPY --from=builder /build/target/release/lwid-server /usr/local/bin/lwid-server

# Environment
ENV LWID_STORAGE__DATA_DIR=/storage
ENV LWID_SERVER__SHELL_DIR=/shell

EXPOSE 8080

USER lwid

ENTRYPOINT ["lwid-server"]
