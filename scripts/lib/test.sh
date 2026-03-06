#!/bin/sh
# Shared test runner — called by per-crate wrapper scripts.
#
# Required env:
#   CRATE_DIR          — absolute path to the crate root (working directory for cargo)
#
# Optional env:
#   CARGO_PKG          — e.g. "--package miru-agent" (empty = default crate)
#   CARGO_FEATURES     — e.g. "--features test"      (empty = no extra features)
#   CARGO_TEST_ARGS    — e.g. "-- --test-threads=1"  (empty = no extra args)
#   RUST_LOG_OVERRIDE  — e.g. "off"                  (empty = inherit RUST_LOG)
set -e

cd "$CRATE_DIR"

if [ -n "$RUST_LOG_OVERRIDE" ]; then
    export RUST_LOG="$RUST_LOG_OVERRIDE"
fi

# shellcheck disable=SC2086
cargo test $CARGO_PKG $CARGO_FEATURES $CARGO_TEST_ARGS
