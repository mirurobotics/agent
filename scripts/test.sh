#!/bin/sh
set -e

# Set the target directory, use the git repo root if no argument provided
git_repo_root_dir=$(git rev-parse --show-toplevel)

cd "$git_repo_root_dir"

# Suppress log output during tests
RUST_LOG=off cargo test --features test -- --test-threads=1