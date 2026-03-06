#!/bin/sh
set -e

git_repo_root_dir=$(git rev-parse --show-toplevel)
LINT_DIR="$git_repo_root_dir/tools/lint"

cd "$LINT_DIR"

# Install cargo-llvm-cov if not available
if ! command -v cargo-llvm-cov >/dev/null 2>&1; then
    echo "Installing cargo-llvm-cov..."
    cargo install cargo-llvm-cov
fi

echo "Generating HTML coverage report..."
cargo llvm-cov --html --output-dir target/coverage

echo ""
echo "Report: target/coverage/html/index.html"
