#!/bin/sh
set -e

git_repo_root_dir=$(git rev-parse --show-toplevel)
cd "$git_repo_root_dir"

# Install cargo-llvm-cov if not available
if ! command -v cargo-llvm-cov >/dev/null 2>&1; then
    echo "Installing cargo-llvm-cov..."
    cargo install cargo-llvm-cov
fi

echo "Generating HTML coverage report..."
RUST_LOG=off cargo llvm-cov --html --output-dir target/coverage \
    --package miru-agent --features test \
    -- --test-threads=1

echo ""
echo "Report: target/coverage/html/index.html"
