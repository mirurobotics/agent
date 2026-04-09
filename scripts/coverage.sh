#!/bin/sh
set -e
REPO_ROOT=$(git rev-parse --show-toplevel)

export CRATE_DIR="$REPO_ROOT"
export CARGO_PKG="--package miru-agent"
export CARGO_FEATURES="--features test"
export CARGO_TEST_ARGS=""
export RUST_LOG_OVERRIDE="off"

exec "$REPO_ROOT/scripts/lib/coverage.sh"
