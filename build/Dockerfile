# Dockerfile - Build miru-agent using the pinned builder image
#
# The builder image is pinned here — update it when Dockerfile.builder changes.
# See Dockerfile.builder for the toolchain definition.
#
# Targets:
#   build     - Full build with artifacts embedded
#   artifacts - Just the build outputs (for --output)
#
# Examples:
#   docker build -f build/Dockerfile --target artifacts --output type=local,dest=./dist .

FROM ghcr.io/mirurobotics/agent-builder:a32d4c0 AS build

COPY . .

# Uses Docker secret to avoid embedding key in image layers
# GORELEASER_ARGS defaults to snapshot for local builds; release.sh overrides it
ARG GORELEASER_PREVIOUS_TAG
ARG GORELEASER_ARGS="--snapshot"
RUN --mount=type=secret,id=GORELEASER_KEY \
    GORELEASER_KEY=$(cat /run/secrets/GORELEASER_KEY) \
    GORELEASER_PREVIOUS_TAG="${GORELEASER_PREVIOUS_TAG}" \
    cd build && goreleaser release ${GORELEASER_ARGS} --clean

# =============================================================================
# Stage: artifacts - Just the build outputs
# =============================================================================
FROM scratch AS artifacts
COPY --from=build /workspace/build/dist/ /
