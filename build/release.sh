#!/bin/sh
set -e

this_repo_root_dir=$(git rev-parse --show-toplevel)
this_dir=$this_repo_root_dir/build
cd "$this_repo_root_dir"

# shellcheck source=git-tags.sh
. "$this_dir/git-tags.sh"
previous_tag=$(previous_tag)
echo "Previous tag: $previous_tag"

# Release build (no --snapshot)
# Builder image is pinned in build/Dockerfile
docker build \
    -f build/Dockerfile \
    --build-arg GORELEASER_PREVIOUS_TAG="$previous_tag" \
    --build-arg GORELEASER_ARGS="" \
    --secret id=GORELEASER_KEY,env=GORELEASER_KEY \
    --secret id=GITHUB_TOKEN,env=GITHUB_TOKEN \
    --target artifacts \
    --output type=local,dest=./build/dist \
    .
