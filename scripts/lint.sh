#!/bin/sh
set -e
REPO_ROOT=$(git rev-parse --show-toplevel)

export CRATE_DIR="$REPO_ROOT"
export CARGO_PKG="--package miru-agent"
export CARGO_CLIPPY_EXTRA="--all-features"
export IMPORT_LINT_PATHS="$REPO_ROOT/agent/src $REPO_ROOT/agent/tests"
export ASSERT_LINT_PATHS="$REPO_ROOT/agent/tests $REPO_ROOT/agent/src/s3/mod.rs $REPO_ROOT/agent/src/s3/multipart.rs $REPO_ROOT/agent/src/gcs/mod.rs $REPO_ROOT/agent/src/sync/syncer.rs $REPO_ROOT/agent/src/data_uploads/upload/transfer.rs $REPO_ROOT/agent/src/data_uploads/upload/executor.rs"
export IMPORT_LINT_CONFIG="$REPO_ROOT/.lint-imports.toml"
export RUN_DIET="1"

exec "$REPO_ROOT/scripts/lib/lint.sh"
