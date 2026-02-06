# Dockerfile.builder - Build environment for miru-agent
#
# Contains Rust, Zig (for cross-compilation via cargo-zigbuild), and GoReleaser.
# Pre-built and pushed to GHCR by the builder.yml CI workflow.
#
# Build locally:
#   docker build -f build/Dockerfile.builder -t miru-agent-builder .
FROM rust:1.93.0-bookworm

LABEL org.opencontainers.image.title="miru-agent-builder"
LABEL org.opencontainers.image.description="Build environment for miru-agent with Rust, Zig, and GoReleaser"
LABEL org.opencontainers.image.source="https://github.com/mirurobotics/agent"

# Install build dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    curl \
    git \
    xz-utils \
    pkg-config \
    libssl-dev \
    minisign \
    && rm -rf /var/lib/apt/lists/*

# Install Zig (required for cargo-zigbuild cross-compilation)
# Verified using minisign with Zig's official public key
ARG ZIG_VERSION=0.15.2
RUN curl -fsSL -o /tmp/zig.tar.xz "https://ziglang.org/download/${ZIG_VERSION}/zig-x86_64-linux-${ZIG_VERSION}.tar.xz" \
    && curl -fsSL -o /tmp/zig.tar.xz.minisig "https://ziglang.org/download/${ZIG_VERSION}/zig-x86_64-linux-${ZIG_VERSION}.tar.xz.minisig" \
    && minisign -Vm /tmp/zig.tar.xz -P RWSGOq2NVecA2UPNdBUZykf1CCb147pkmdtYxgb3Ti+JO/wCYvhbAb/U \
    && tar -xJf /tmp/zig.tar.xz -C /opt \
    && ln -s /opt/zig-x86_64-linux-${ZIG_VERSION}/zig /usr/local/bin/zig \
    && rm /tmp/zig.tar.xz /tmp/zig.tar.xz.minisig

# Install cargo-zigbuild
RUN cargo install cargo-zigbuild

# Install GoReleaser (OSS version - Pro features unlocked via GORELEASER_KEY at runtime)
# Verified using SHA256 checksum from release
ARG GORELEASER_VERSION=2.13.3
RUN curl -fsSL -o /tmp/goreleaser_Linux_x86_64.tar.gz "https://github.com/goreleaser/goreleaser/releases/download/v${GORELEASER_VERSION}/goreleaser_Linux_x86_64.tar.gz" \
    && curl -fsSL -o /tmp/checksums.txt "https://github.com/goreleaser/goreleaser/releases/download/v${GORELEASER_VERSION}/checksums.txt" \
    && cd /tmp && grep "goreleaser_Linux_x86_64.tar.gz$" checksums.txt | sha256sum -c - \
    && tar -xzf /tmp/goreleaser_Linux_x86_64.tar.gz -C /usr/local/bin goreleaser \
    && rm /tmp/goreleaser_Linux_x86_64.tar.gz /tmp/checksums.txt \
    && goreleaser --version

# Add Rust targets for cross-compilation
RUN rustup target add x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu

WORKDIR /workspace
CMD ["bash"]

