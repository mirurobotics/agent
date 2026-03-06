#!/bin/sh
set -e

# Set the target directory, use the git repo root if no argument provided
git_repo_root_dir=$(git rev-parse --show-toplevel)
LINT_DIR="$git_repo_root_dir/tools/lint"

cd "$LINT_DIR"

cargo update --verbose
echo ""
