#!/bin/sh
# Update Cargo dependencies (refreshes Cargo.lock).
set -e
REPO_ROOT=$(git rev-parse --show-toplevel)
cd "$REPO_ROOT"

echo "Updating Cargo dependencies"
echo "---------------------------"
cargo update --verbose
echo ""
