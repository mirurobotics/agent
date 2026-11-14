#!/bin/sh
set -e
REPO_ROOT=$(git rev-parse --show-toplevel)

export CRATE_DIR="$REPO_ROOT"
export SRC_DIR="daemon/src"
export CARGO_PKG="--package miru-daemon"
export CARGO_FEATURES=""
export RUST_LOG_OVERRIDE="off"

exec "$REPO_ROOT/scripts/lib/covgate.sh"
