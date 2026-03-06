#!/bin/sh
# Shared linter — called by per-crate wrapper scripts.
#
# Runs: cargo fmt, import linter, machete, audit, clippy.
# Optionally: cargo diet, cargo update.
#
# Required env:
#   CRATE_DIR            — absolute path to the crate root (working directory for cargo)
#   IMPORT_LINT_PATHS    — space-separated paths to lint with the import linter
#   IMPORT_LINT_CONFIG   — path to .lint-imports.toml
#
# Optional env:
#   CARGO_PKG            — e.g. "--package miru-agent" (empty = default crate)
#   CARGO_CLIPPY_EXTRA   — e.g. "--all-features"      (empty = no extra flags)
#   LINT_FIX             — 1 = auto-fix (default), 0 = check-only (CI)
#   RUN_DIET             — 1 = run cargo diet, 0 = skip (default)
set -e

REPO_ROOT=$(git rev-parse --show-toplevel)
cd "$CRATE_DIR"

LINT_FIX="${LINT_FIX:-1}"
RUN_DIET="${RUN_DIET:-0}"

# Update cargo dependencies (skip in check-only mode)
if [ "$LINT_FIX" = "1" ]; then
    echo "Updating the Cargo dependencies"
    echo "-------------------------------"
    cargo update --verbose
    echo ""
fi

echo "Cargo fmt"
echo "---------"
if [ "$LINT_FIX" = "1" ]; then
    # shellcheck disable=SC2086
    cargo fmt $CARGO_PKG
else
    # shellcheck disable=SC2086
    cargo fmt $CARGO_PKG -- --check
fi
echo ""

echo "Custom Linter"
echo "-------------"
for lint_path in $IMPORT_LINT_PATHS; do
    if [ "$LINT_FIX" = "1" ]; then
        cargo run --manifest-path "$REPO_ROOT/tools/lint/Cargo.toml" -- --path "$lint_path" --fix --config "$IMPORT_LINT_CONFIG"
    else
        cargo run --manifest-path "$REPO_ROOT/tools/lint/Cargo.toml" -- --path "$lint_path" --config "$IMPORT_LINT_CONFIG"
    fi
done
echo ""

echo "Unused external dependencies"
echo "----------------------------"
cargo machete
echo ""

if [ "$RUN_DIET" = "1" ]; then
    echo "Unused internal code"
    echo "--------------------"
    cargo diet
    echo ""
fi

echo "Security vulnerabilities"
echo "------------------------"
cargo audit
echo ""

echo "Clippy"
echo "------"
if [ "$LINT_FIX" = "1" ]; then
    # shellcheck disable=SC2086
    cargo clippy $CARGO_PKG --fix --allow-dirty $CARGO_CLIPPY_EXTRA -- -D warnings
fi
# shellcheck disable=SC2086
cargo clippy $CARGO_PKG --no-deps $CARGO_CLIPPY_EXTRA -- -D warnings
echo ""

echo "Lint complete"
echo ""
