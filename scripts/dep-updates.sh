#!/bin/sh
set -e
REPO_ROOT=$(git rev-parse --show-toplevel)

export CRATE_DIR="${1:-$REPO_ROOT}"

exec "$REPO_ROOT/scripts/lib/dep-updates.sh"
