FROM ubuntu:24.04 AS base

ENV DEBIAN_FRONTEND=noninteractive
WORKDIR /tmp/

RUN apt-get update && apt-get install -y \
    build-essential \
    ca-certificates \
    curl \
    git \
    && rm -rf /var/lib/apt/lists/*

ARG RUST_VERSION="1.87.0"
ENV RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:$PATH

# Toolchain is world-writable because Jenkins runs the container with the host uid.
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs -o /tmp/rustup-init.sh \
    && sh /tmp/rustup-init.sh -y --no-modify-path --profile minimal --default-toolchain $RUST_VERSION \
    && rm /tmp/rustup-init.sh \
    && rustup component add clippy \
    && chmod -R a+w $RUSTUP_HOME $CARGO_HOME

FROM base AS static-analysis
RUN rustup component add rustfmt \
    && chmod -R a+w $RUSTUP_HOME $CARGO_HOME

FROM base AS native

FROM base AS native-i686
RUN apt-get update && apt-get install -y \
    gcc-multilib \
    && rm -rf /var/lib/apt/lists/* \
    && rustup target add i686-unknown-linux-gnu \
    && chmod -R a+w $RUSTUP_HOME $CARGO_HOME

FROM base AS cross
RUN rustup target add \
    thumbv6m-none-eabi \
    thumbv7em-none-eabihf \
    riscv32imac-unknown-none-elf \
    aarch64-unknown-none \
    powerpc64-unknown-linux-gnu \
    && chmod -R a+w $RUSTUP_HOME $CARGO_HOME
