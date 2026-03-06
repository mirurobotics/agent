#!/bin/sh
# Shared HTML coverage report generator — called by per-crate wrapper scripts.
#
# Required env:
#   CRATE_DIR          — absolute path to the crate root
#
# Optional env:
#   CARGO_PKG          — e.g. "--package miru-agent"
#   CARGO_FEATURES     — e.g. "--features test"
#   CARGO_TEST_ARGS    — e.g. "-- --test-threads=1"
#   RUST_LOG_OVERRIDE  — e.g. "off"
set -e

cd "$CRATE_DIR"

if ! command -v cargo-llvm-cov >/dev/null 2>&1; then
    echo "Installing cargo-llvm-cov..."
    cargo install cargo-llvm-cov
fi

if [ -n "$RUST_LOG_OVERRIDE" ]; then
    export RUST_LOG="$RUST_LOG_OVERRIDE"
fi

echo "Generating HTML coverage report..."
# shellcheck disable=SC2086
cargo llvm-cov --html --output-dir target/coverage \
    $CARGO_PKG \
    $CARGO_FEATURES \
    $CARGO_TEST_ARGS

echo ""
echo "Report: target/coverage/html/index.html"
