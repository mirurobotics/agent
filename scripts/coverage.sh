#!/bin/sh
set -e

git_repo_root_dir=$(git rev-parse --show-toplevel)
cd "$git_repo_root_dir"

# Install cargo-llvm-cov if not available
if ! command -v cargo-llvm-cov >/dev/null 2>&1; then
    echo "Installing cargo-llvm-cov..."
    cargo install cargo-llvm-cov
fi

MODULE="$1"

if [ -n "$MODULE" ]; then
    echo "Generating HTML coverage report for module: $MODULE"
    RUST_LOG=off cargo llvm-cov --html --output-dir target/coverage \
        --package miru-agent --features test \
        -- --test-threads=1 "::${MODULE}::"
else
    echo "Generating HTML coverage report for all modules..."
    RUST_LOG=off cargo llvm-cov --html --output-dir target/coverage \
        --package miru-agent --features test \
        -- --test-threads=1
fi

echo ""
echo "Report: target/coverage/html/index.html"
