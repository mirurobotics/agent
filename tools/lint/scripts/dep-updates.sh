#!/bin/sh
set -e
REPO_ROOT=$(git rev-parse --show-toplevel)

export CRATE_DIR="$REPO_ROOT/tools/lint"

exec "$REPO_ROOT/scripts/lib/dep-updates.sh"
