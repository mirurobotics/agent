#!/bin/sh
set -e

this_repo_root_dir=$(git rev-parse --show-toplevel)
this_dir=$this_repo_root_dir/build
cd "$this_repo_root_dir"

# shellcheck source=./git-tags.sh
. "$this_dir/git-tags.sh"
previous_tag=$(previous_tag)
echo "Previous tag: $previous_tag"

# Pre-built builder image for faster builds
BUILDER_IMAGE="${BUILDER_IMAGE:-ghcr.io/mirurobotics/agent-builder:sha}"

# Pull the builder image (ignore failures for offline/first-time builds)
echo "Pulling builder image: $BUILDER_IMAGE"
docker pull "$BUILDER_IMAGE" 2>/dev/null || echo "Warning: Could not pull builder image, will build from scratch"

# Build using Docker and extract artifacts to ./build/dist
# Uses --cache-from to leverage the pre-built builder image layers
docker build \
    -f build/Dockerfile \
    --build-arg GORELEASER_PREVIOUS_TAG="$previous_tag" \
    --secret id=GORELEASER_KEY,env=GORELEASER_KEY \
    --cache-from "$BUILDER_IMAGE" \
    --target artifacts \
    --output type=local,dest=./build/dist \
    .
