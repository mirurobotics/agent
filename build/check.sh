#!/bin/sh
set -e

this_repo_root_dir=$(git rev-parse --show-toplevel)
cd "$this_repo_root_dir"

# Validate the release config. Builder image is pinned in build/Dockerfile.
docker build \
    -f build/Dockerfile \
    --target check \
    .
