#!/bin/sh
# Shared HTML coverage report generator — called by per-crate wrapper scripts.
#
# Required env:
#   CRATE_DIR          — absolute path to the crate root
#
# Optional env:
#   CARGO_PKG          — e.g. "--package miru-agent"
#   CARGO_FEATURES     — optional Cargo feature flags
#   CARGO_TEST_ARGS    — e.g. "-- --test-threads=1"
#   RUST_LOG_OVERRIDE  — e.g. "off"
#
# Usage: coverage.sh [--report-only]
# Report-only mode uses previously recorded coverage without running tests.
set -e

if [ "$#" -gt 1 ] || { [ "$#" -eq 1 ] && [ "$1" != "--report-only" ]; }; then
    echo "Usage: $0 [--report-only]" >&2
    exit 2
fi
report_only=false
if [ "${1:-}" = "--report-only" ]; then
    report_only=true
fi

cd "$CRATE_DIR"

if ! command -v cargo-llvm-cov >/dev/null 2>&1; then
    echo "Installing cargo-llvm-cov..."
    cargo install cargo-llvm-cov
fi

if [ -n "$RUST_LOG_OVERRIDE" ]; then
    export RUST_LOG="$RUST_LOG_OVERRIDE"
fi

echo "Generating HTML coverage report..."
set -- cargo llvm-cov
if "$report_only"; then
    set -- "$@" report
fi
# shellcheck disable=SC2086
set -- "$@" --html --output-dir target/coverage $CARGO_PKG $CARGO_FEATURES
if ! "$report_only"; then
    # shellcheck disable=SC2086
    set -- "$@" $CARGO_TEST_ARGS
fi
"$@"

echo ""
echo "Report: target/coverage/html/index.html"
