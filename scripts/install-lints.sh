#!/bin/sh
# Install/update lint tooling dependencies.
# Run this once before running lint.sh or preflight.sh.
set -e
REPO_ROOT=$(git rev-parse --show-toplevel)
cd "$REPO_ROOT"

echo "Updating Cargo dependencies"
echo "---------------------------"
cargo update --verbose
echo ""

echo "Install complete"
