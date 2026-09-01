#!/bin/sh
set -e
REPO_ROOT=$(git rev-parse --show-toplevel)

export CRATE_DIR="$REPO_ROOT"
export CARGO_PKG="--package miru-agent"
export CARGO_CLIPPY_EXTRA="--all-features"
export IMPORT_LINT_PATHS="$REPO_ROOT/agent/src $REPO_ROOT/agent/tests"
export ASSERT_LINT_PATHS="$REPO_ROOT/agent/tests"
export IMPORT_LINT_CONFIG="$REPO_ROOT/.lint-imports.toml"
export RUN_DIET="1"

"$REPO_ROOT/scripts/lib/lint.sh"

# Surface lint (YAML / shell / GitHub Actions) is enforced in CI by a dedicated
# job (.github/workflows/ci.yml: surface-lint, via the shared reusable
# workflow), which installs its own toolchain. Run it here only for local
# developer invocations so the Rust lint CI job need not install the surface
# tools. Mirrors the CI guard in scripts/lib/install-lint-deps.sh.
if [ "$CI" != "true" ]; then
    "$REPO_ROOT/scripts/lint-surface.sh"
fi
