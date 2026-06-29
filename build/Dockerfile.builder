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

# Install cargo-auditable so release builds embed each binary's dependency tree
# in a `.dep-v0` ELF section. syft (>= 1.15) reads that section, so SBOMs
# generated from the binaries/archives list every linked crate instead of just
# the top-level package. Wired up in build/.goreleaser.yaml via the Rust
# builder's `tool:` (see the wrapper below).
RUN cargo install cargo-auditable --locked

# Wrapper used as GoReleaser's Rust build `tool:`. cargo-auditable only activates
# when invoked as `cargo auditable <cmd>` — it inspects argv[1] and refuses to run
# if it isn't "auditable" — so GoReleaser's `tool:`/`command:` cannot point at the
# cargo-auditable binary directly. This wrapper restores the `cargo auditable`
# invocation while passing through whatever GoReleaser appends (e.g.
# `zigbuild --target=... --release -p=miru-agent`).
RUN printf '#!/usr/bin/env bash\nexec cargo auditable "$@"\n' \
        > /usr/local/bin/cargo-auditable-zigbuild \
    && chmod +x /usr/local/bin/cargo-auditable-zigbuild

# Install GoReleaser (OSS version - Pro features unlocked via GORELEASER_KEY at runtime)
# Verified using SHA256 checksum from release
ARG GORELEASER_VERSION=2.13.3
RUN curl -fsSL -o /tmp/goreleaser_Linux_x86_64.tar.gz "https://github.com/goreleaser/goreleaser/releases/download/v${GORELEASER_VERSION}/goreleaser_Linux_x86_64.tar.gz" \
    && curl -fsSL -o /tmp/checksums.txt "https://github.com/goreleaser/goreleaser/releases/download/v${GORELEASER_VERSION}/checksums.txt" \
    && cd /tmp && grep "goreleaser_Linux_x86_64.tar.gz$" checksums.txt | sha256sum -c - \
    && tar -xzf /tmp/goreleaser_Linux_x86_64.tar.gz -C /usr/local/bin goreleaser \
    && rm /tmp/goreleaser_Linux_x86_64.tar.gz /tmp/checksums.txt \
    && goreleaser --version

# Install syft (Anchore) for SBOM generation. build/.goreleaser.yaml's
# `sboms:` stanza shells out to `syft` (GoReleaser's default SBOM cmd) to
# produce SPDX-JSON SBOMs for the archives and the .deb package.
# Verified using the SHA256 checksum from the syft release.
ARG SYFT_VERSION=1.46.0
RUN curl -fsSL -o /tmp/syft_${SYFT_VERSION}_linux_amd64.tar.gz "https://github.com/anchore/syft/releases/download/v${SYFT_VERSION}/syft_${SYFT_VERSION}_linux_amd64.tar.gz" \
    && curl -fsSL -o /tmp/syft_checksums.txt "https://github.com/anchore/syft/releases/download/v${SYFT_VERSION}/syft_${SYFT_VERSION}_checksums.txt" \
    && cd /tmp && grep "syft_${SYFT_VERSION}_linux_amd64.tar.gz$" syft_checksums.txt | sha256sum -c - \
    && tar -xzf /tmp/syft_${SYFT_VERSION}_linux_amd64.tar.gz -C /usr/local/bin syft \
    && rm /tmp/syft_${SYFT_VERSION}_linux_amd64.tar.gz /tmp/syft_checksums.txt \
    && syft version

# Add Rust targets for cross-compilation
RUN rustup target add x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu

WORKDIR /workspace
CMD ["bash"]

