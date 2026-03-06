#!/bin/sh
set -e
REPO_ROOT=$(git rev-parse --show-toplevel)
LINT_DIR="$REPO_ROOT/tools/lint"

export CRATE_DIR="$LINT_DIR"
export IMPORT_LINT_PATHS="$LINT_DIR/src"
export IMPORT_LINT_CONFIG="$LINT_DIR/.lint-imports.toml"

exec "$REPO_ROOT/scripts/lib/lint.sh"
