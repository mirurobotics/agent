# Dockerfile - Build miru-agent using the pinned builder image
#
# The builder image is pinned here by digest, because its tag can be re-pushed.
# When Dockerfile.builder changes, update both the tag and the digest.
# See Dockerfile.builder for the toolchain definition.
#
# Targets:
#   check     - Validate the GoReleaser config
#   build     - Full build with artifacts embedded
#   artifacts - Just the build outputs (for --output)
#
# Examples:
#   docker build -f build/Dockerfile --target check .
#   docker build -f build/Dockerfile --target artifacts --output type=local,dest=./dist .

FROM ghcr.io/mirurobotics/agent-builder:0d2ac7e@sha256:79e78a13ba40bbabeffac2fb7d92d8031f24ebc779e1043ef05cf6114699b915 AS source

COPY . .

# =============================================================================
# Stage: check - Validate the GoReleaser config
# =============================================================================
FROM source AS check
RUN cd build && goreleaser check

# =============================================================================
# Stage: build - Build the release artifacts
# =============================================================================
FROM source AS build

# Uses Docker secrets to avoid embedding keys in image layers
# GORELEASER_ARGS defaults to snapshot for local builds; release.sh overrides it
ARG GORELEASER_PREVIOUS_TAG
ARG GORELEASER_ARGS="--snapshot"
RUN --mount=type=secret,id=GORELEASER_KEY \
    --mount=type=secret,id=GITHUB_TOKEN \
    export GORELEASER_KEY="$(cat /run/secrets/GORELEASER_KEY 2>/dev/null || true)" && \
    export GITHUB_TOKEN="$(cat /run/secrets/GITHUB_TOKEN 2>/dev/null || true)" && \
    export GORELEASER_PREVIOUS_TAG="${GORELEASER_PREVIOUS_TAG}" && \
    cd build && goreleaser release ${GORELEASER_ARGS} --clean

# =============================================================================
# Stage: artifacts - Just the build outputs
# =============================================================================
FROM scratch AS artifacts
COPY --from=build /workspace/build/dist/ /
