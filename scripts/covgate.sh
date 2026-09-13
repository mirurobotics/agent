#!/bin/sh
set -e
REPO_ROOT=$(git rev-parse --show-toplevel)

export CRATE_DIR="$REPO_ROOT"
export SRC_DIR="agent/src"
export CARGO_PKG="--package miru-agent"
export CARGO_FEATURES=""
export RUST_LOG_OVERRIDE="off"
export COV_IGNORE_FILENAME_REGEX='/agent/src/(.*/)?tests/'

exec "$REPO_ROOT/scripts/lib/covgate.sh"
