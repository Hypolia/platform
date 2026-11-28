# syntax=docker/dockerfile:1

# Build contexts:
#   .            this repository
#   sdk-src      a checkout of hypolia/sdk at sdk.ref
#   rspigot-src  a checkout of ferrislabs/rspigot at rspigot.ref

ARG RUST_VERSION=1.90
ARG DEBIAN_RUNTIME_IMAGE=debian:trixie-slim
ARG DISTROLESS_IMAGE=gcr.io/distroless/cc-debian13:nonroot

FROM rust:${RUST_VERSION}-trixie AS chef

ENV RUSTUP_TOOLCHAIN=stable \
    CARGO_TERM_COLOR=always

RUN apt-get update && apt-get install -y --no-install-recommends \
        protobuf-compiler \
        pkg-config \
        libssl-dev \
    && rm -rf /var/lib/apt/lists/*

RUN --mount=type=cache,id=hypolia-cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=hypolia-cargo-git,target=/usr/local/cargo/git \
    cargo install cargo-chef --version 0.1.77 --locked

# ==============================================================================
# Platform workspace: orchestrator, map-sync, migrations, Hypolia proxy plugin
# ==============================================================================
FROM chef AS platform-src

COPY --exclude=target --exclude=.git --exclude=rust-toolchain.toml --from=rspigot-src . /opt/minecraft/dev/rspigot
COPY --exclude=target --exclude=.git --from=sdk-src . /src/sdk
WORKDIR /src/platform
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates

FROM platform-src AS platform-planner

RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS platform-builder

COPY --exclude=target --exclude=.git --exclude=rust-toolchain.toml --from=rspigot-src . /opt/minecraft/dev/rspigot
COPY --exclude=target --exclude=.git --from=sdk-src . /src/sdk
WORKDIR /src/platform
COPY --from=platform-planner /src/platform/recipe.json recipe.json
RUN --mount=type=cache,id=hypolia-cargo-registry-platform,sharing=locked,target=/usr/local/cargo/registry \
    --mount=type=cache,id=hypolia-cargo-git-platform,sharing=locked,target=/usr/local/cargo/git \
    cargo chef cook --release --recipe-path recipe.json

COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN --mount=type=cache,id=hypolia-cargo-registry-platform,sharing=locked,target=/usr/local/cargo/registry \
    --mount=type=cache,id=hypolia-cargo-git-platform,sharing=locked,target=/usr/local/cargo/git \
    cargo build --release -p hypolia-orchestrator --bin hypolia-orchestrator --bin hypolia-map-sync \
    && cargo build --release -p hypolia-progression-pg --bin hypolia-migrate \
    && cargo build --release -p hypolia-proxy

FROM ${DEBIAN_RUNTIME_IMAGE} AS orchestrator

RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates \
        unzip \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --create-home --home-dir /app hypolia

WORKDIR /app
COPY --from=platform-builder /src/platform/target/release/hypolia-orchestrator /usr/local/bin/hypolia-orchestrator
COPY --from=platform-builder /src/platform/target/release/hypolia-map-sync /usr/local/bin/hypolia-map-sync
COPY --from=platform-builder /src/platform/target/release/hypolia-migrate /usr/local/bin/hypolia-migrate

USER hypolia
EXPOSE 8080
ARG HYPOLIA_VERSION=dev
ENV HYPOLIA_VERSION=${HYPOLIA_VERSION}
ENTRYPOINT ["/usr/local/bin/hypolia-orchestrator"]

FROM ${DISTROLESS_IMAGE} AS proxy-plugin

COPY --from=platform-builder /src/platform/target/release/hypolia-proxy /usr/local/bin/hypolia-proxy
ARG HYPOLIA_VERSION=dev
ENV HYPOLIA_VERSION=${HYPOLIA_VERSION}
ENTRYPOINT ["/usr/local/bin/hypolia-proxy"]

# ==============================================================================
# rspigot-proxy, built from ferrislabs/rspigot at rspigot.ref
# ==============================================================================
FROM chef AS proxy-planner

WORKDIR /src/rspigot
COPY --exclude=target --exclude=.git --exclude=rust-toolchain.toml --from=rspigot-src . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS proxy-builder

WORKDIR /src/rspigot
COPY --from=proxy-planner /src/rspigot/recipe.json recipe.json
RUN --mount=type=cache,id=hypolia-cargo-registry-proxy,sharing=locked,target=/usr/local/cargo/registry \
    --mount=type=cache,id=hypolia-cargo-git-proxy,sharing=locked,target=/usr/local/cargo/git \
    --mount=type=cache,id=hypolia-cargo-target-proxy,sharing=locked,target=/src/rspigot/target \
    cargo chef cook --release -p rspigot-proxy --recipe-path recipe.json

COPY --exclude=target --exclude=.git --exclude=rust-toolchain.toml --from=rspigot-src . .
RUN --mount=type=cache,id=hypolia-cargo-registry-proxy,sharing=locked,target=/usr/local/cargo/registry \
    --mount=type=cache,id=hypolia-cargo-git-proxy,sharing=locked,target=/usr/local/cargo/git \
    --mount=type=cache,id=hypolia-cargo-target-proxy,sharing=locked,target=/src/rspigot/target \
    cargo build --release -p rspigot-proxy \
    && cp /src/rspigot/target/release/rspigot-proxy /usr/local/bin/rspigot-proxy

FROM ${DISTROLESS_IMAGE} AS proxy

COPY --from=proxy-builder /usr/local/bin/rspigot-proxy /usr/local/bin/rspigot-proxy
EXPOSE 25565 25579 9100
ARG RSPIGOT_REF=local
ENV RSPIGOT_REF=${RSPIGOT_REF}
STOPSIGNAL SIGTERM
ENTRYPOINT ["/usr/local/bin/rspigot-proxy", "/etc/rspigot-proxy"]
