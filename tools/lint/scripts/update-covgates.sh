#!/bin/sh
set -e
REPO_ROOT=$(git rev-parse --show-toplevel)

export CRATE_DIR="$REPO_ROOT/tools/lint"
export SRC_DIR="src"

exec "$REPO_ROOT/scripts/lib/update-covgates.sh"
