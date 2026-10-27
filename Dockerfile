FROM rust:1-bookworm AS builder

WORKDIR /build

RUN apt-get update \
    && apt-get install --no-install-recommends -y pkg-config libdbus-1-dev \
    && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock ./
COPY src ./src

RUN cargo build --locked --release --bin somaris

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install --no-install-recommends -y ca-certificates libdbus-1-3 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --create-home somaris \
    && mkdir /data \
    && chown somaris:somaris /data

COPY --from=builder --chown=somaris:somaris /build/target/release/somaris /usr/local/bin/somaris

WORKDIR /app
USER somaris

ENV SOMARIS_LISTEN=0.0.0.0:8080 \
    SOMARIS_DATA_DIR=/data \
    RUST_LOG=info

EXPOSE 8080
VOLUME ["/data"]

ENTRYPOINT ["/usr/local/bin/somaris"]
