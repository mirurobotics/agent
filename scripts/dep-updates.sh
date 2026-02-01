#!/bin/sh
set -e 

# Set the target directory, use the git repo root if no argument provided
git_repo_root_dir=$(git rev-parse --show-toplevel)
TARGET_DIR="${1:-$git_repo_root_dir}"
cd "$TARGET_DIR"

cargo update --verbose
echo ""