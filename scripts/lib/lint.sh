#!/bin/sh
# Shared linter — called by per-crate wrapper scripts.
#
# Runs: import linter, cargo fmt, machete, audit, clippy.
# Optionally: cargo diet.
# Run scripts/update-deps.sh first to refresh Cargo.lock.
#
# Required env:
#   CRATE_DIR            — absolute path to the crate root (working directory for cargo)
#   IMPORT_LINT_PATHS    — space-separated paths to lint with the import linter
#   IMPORT_LINT_CONFIG   — path to .lint-imports.toml
#
# Optional env:
#   CARGO_PKG            — e.g. "--package miru-daemon" (empty = default crate)
#   CARGO_CLIPPY_EXTRA   — e.g. "--all-features"      (empty = no extra flags)
#   LINT_FIX             — 1 = auto-fix (default), 0 = check-only (CI)
#   RUN_DIET             — 1 = run cargo diet, 0 = skip (default)
set -e

cd "$CRATE_DIR"

LINT_FIX="${LINT_FIX:-1}"
RUN_DIET="${RUN_DIET:-0}"

echo "Custom Linter"
echo "-------------"
cargo install --quiet --locked --git https://github.com/mirurobotics/rusttools --rev b0534239fff834429fda35f6db13a6ef64dacc2a lint-imports
ASSERT_ARGS=""
if [ -n "${ASSERT_LINT_PATHS:-}" ]; then
    ASSERT_ARGS="--assert-paths $ASSERT_LINT_PATHS"
fi
for lint_path in $IMPORT_LINT_PATHS; do
    if [ "$LINT_FIX" = "1" ]; then
        # shellcheck disable=SC2086
        lint-imports --path "$lint_path" --fix --config "$IMPORT_LINT_CONFIG" $ASSERT_ARGS
    else
        # shellcheck disable=SC2086
        lint-imports --path "$lint_path" --config "$IMPORT_LINT_CONFIG" $ASSERT_ARGS
    fi
    # Only run assert check on the first iteration to avoid duplicate reports.
    ASSERT_ARGS=""
done
echo ""

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
    cargo clippy $CARGO_PKG --fix --allow-dirty --all-targets $CARGO_CLIPPY_EXTRA -- -D warnings
fi
# shellcheck disable=SC2086
cargo clippy $CARGO_PKG --no-deps --all-targets $CARGO_CLIPPY_EXTRA -- -D warnings
echo ""

echo "Lint complete"
echo ""
