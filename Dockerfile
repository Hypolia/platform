# syntax=docker/dockerfile:1

# Build contexts:
#   .        this repository
#   sdk-src  a checkout of hypolia/sdk at sdk.ref

ARG RUST_VERSION=1.90
ARG DEBIAN_RUNTIME_IMAGE=debian:trixie-slim

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
# Orchestrator, map-sync and progression migrations
# ==============================================================================
FROM chef AS orchestrator-src

COPY --exclude=target --exclude=.git --from=sdk-src . /src/sdk
WORKDIR /src/platform
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates

FROM orchestrator-src AS orchestrator-planner

RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS orchestrator-builder

COPY --exclude=target --exclude=.git --from=sdk-src . /src/sdk
WORKDIR /src/platform
COPY --from=orchestrator-planner /src/platform/recipe.json recipe.json
RUN --mount=type=cache,id=hypolia-cargo-registry-platform,sharing=locked,target=/usr/local/cargo/registry \
    --mount=type=cache,id=hypolia-cargo-git-platform,sharing=locked,target=/usr/local/cargo/git \
    cargo chef cook --release --recipe-path recipe.json

COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN --mount=type=cache,id=hypolia-cargo-registry-platform,sharing=locked,target=/usr/local/cargo/registry \
    --mount=type=cache,id=hypolia-cargo-git-platform,sharing=locked,target=/usr/local/cargo/git \
    cargo build --release -p hypolia-orchestrator --bin hypolia-orchestrator --bin hypolia-map-sync \
    && cargo build --release -p hypolia-progression-pg --bin hypolia-migrate

FROM ${DEBIAN_RUNTIME_IMAGE} AS orchestrator

RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates \
        unzip \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --create-home --home-dir /app hypolia

WORKDIR /app
COPY --from=orchestrator-builder /src/platform/target/release/hypolia-orchestrator /usr/local/bin/hypolia-orchestrator
COPY --from=orchestrator-builder /src/platform/target/release/hypolia-map-sync /usr/local/bin/hypolia-map-sync
COPY --from=orchestrator-builder /src/platform/target/release/hypolia-migrate /usr/local/bin/hypolia-migrate

USER hypolia
EXPOSE 8080
ARG HYPOLIA_VERSION=dev
ENV HYPOLIA_VERSION=${HYPOLIA_VERSION}
ENTRYPOINT ["/usr/local/bin/hypolia-orchestrator"]

# ==============================================================================
# Velocity proxy + Hypolia Velocity plugin
# ==============================================================================
FROM maven:3.9-eclipse-temurin-25 AS velocity-builder

WORKDIR /build
ARG VELOCITY_URL="https://fill-data.papermc.io/v1/objects/0844bcabdcdb4f5b875906fd4c3365bc1bb76918e42bc595ed88ad195258dd75/velocity-4.3.1-45.jar"
ARG VELOCITY_SHA256="0844bcabdcdb4f5b875906fd4c3365bc1bb76918e42bc595ed88ad195258dd75"

RUN curl -fSL -o /build/velocity.jar "${VELOCITY_URL}" \
    && echo "${VELOCITY_SHA256}  /build/velocity.jar" | sha256sum -c -

COPY proxy/velocity-plugin/pom.xml ./
RUN --mount=type=cache,id=hypolia-maven-repo,target=/root/.m2 \
    mvn -B dependency:go-offline

COPY proxy/velocity-plugin/src ./src
RUN --mount=type=cache,id=hypolia-maven-repo,target=/root/.m2 \
    mvn -B package

FROM eclipse-temurin:25-jre-noble AS velocity

RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=velocity-builder /build/velocity.jar /app/velocity.jar
COPY --from=velocity-builder /build/target/hypolia-velocity.jar /app/plugins/hypolia-velocity.jar
COPY --chmod=0755 deploy/docker/velocity-entrypoint.sh /usr/local/bin/velocity-entrypoint.sh

EXPOSE 25565
ARG HYPOLIA_VERSION=dev
ENV HYPOLIA_VERSION=${HYPOLIA_VERSION}
ENTRYPOINT ["/usr/local/bin/velocity-entrypoint.sh"]
