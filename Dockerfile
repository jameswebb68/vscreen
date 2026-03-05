# Stage 1: Build
FROM rust:bookworm AS builder

RUN apt-get update && apt-get install -y \
    libvpx-dev libopus-dev libpulse-dev nasm pkg-config \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates/ crates/

RUN cargo build --release --bin vscreen

# Stage 2: Runtime
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    xvfb pulseaudio chromium \
    libvpx7 libopus0 libpulse0 \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/vscreen /usr/local/bin/vscreen

EXPOSE 8450

ENV VSCREEN_LISTEN=0.0.0.0:8450
ENV VSCREEN_LOG_LEVEL=info

ENTRYPOINT ["vscreen", "--dev"]
