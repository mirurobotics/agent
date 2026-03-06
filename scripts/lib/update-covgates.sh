#!/bin/sh
# Shared coverage threshold ratchet — called by per-crate wrapper scripts.
#
# Discovers .covgate files under SRC_DIR, runs tests with coverage
# instrumentation, and ratchets thresholds up when actual coverage exceeds
# the current threshold (never lowers).
#
# Required env:
#   CRATE_DIR  — absolute path to the crate root
#   SRC_DIR    — path to source directory (relative to CRATE_DIR) containing .covgate files
#
# Optional env:
#   CARGO_PKG          — e.g. "--package miru-agent"
#   CARGO_FEATURES     — e.g. "--features test"
#   CARGO_TEST_ARGS    — e.g. "-- --test-threads=1"
#   RUST_LOG_OVERRIDE  — e.g. "off"
set -e

cd "$CRATE_DIR"

# Check dependencies
if ! command -v jq >/dev/null 2>&1; then
    echo "ERROR: jq is required but not installed."
    echo "  macOS:  brew install jq"
    echo "  Ubuntu: sudo apt-get install jq"
    exit 1
fi

if ! command -v cargo-llvm-cov >/dev/null 2>&1; then
    echo "Installing cargo-llvm-cov..."
    cargo install cargo-llvm-cov
fi

if [ -n "$RUST_LOG_OVERRIDE" ]; then
    export RUST_LOG="$RUST_LOG_OVERRIDE"
fi

echo "Running tests with coverage instrumentation..."
echo ""

# Run cargo-llvm-cov once, capture JSON output
set +e
# shellcheck disable=SC2086
COV_JSON=$(cargo llvm-cov --json $CARGO_PKG $CARGO_FEATURES $CARGO_TEST_ARGS)
TEST_EXIT=$?
set -e

if [ "$TEST_EXIT" -ne 0 ]; then
    echo ""
    echo "ERROR: tests failed (exit $TEST_EXIT) — fix failing tests before updating thresholds"
    exit 1
fi

echo ""
echo "Updating .covgate files (ratchet up only)..."
echo ""

CRATE_DIR_ABS=$(pwd)
UPDATED=0
UNCHANGED=0

# Discover modules: every directory under SRC_DIR that contains a .covgate file
covgate_list=$(mktemp)
trap 'rm -f "$covgate_list"' EXIT
find "$SRC_DIR" -name '.covgate' -type f | sort > "$covgate_list"

while read -r covgate_file; do
    module_path=$(dirname "$covgate_file")
    module_display="${module_path#$SRC_DIR/}"
    # If .covgate is directly in SRC_DIR, display as the directory name
    if [ "$module_display" = "$module_path" ]; then
        module_display=$(basename "$SRC_DIR")
    fi
    module_dir_abs="$CRATE_DIR_ABS/$module_path/"

    current=$(head -1 "$covgate_file" | tr -d '[:space:]')

    actual=$(echo "$COV_JSON" | jq -r --arg mod "$module_dir_abs" '
        [.data[0].files[] | select(.filename | startswith($mod))] |
        if length == 0 then "0.0"
        else
            (map(.summary.regions.count) | add) as $total |
            (map(.summary.regions.covered) | add) as $covered |
            if $total == 0 then "0.0"
            else (($covered / $total) * 10000 | floor) / 100 | tostring
            end
        end
    ')

    # Skip modules with threshold 0 (opted out)
    if [ "$current" = "0" ]; then
        echo "⏭️  ${module_display}: skipped (threshold: 0)"
        UNCHANGED=$((UNCHANGED + 1))
        continue
    fi

    # Ratchet up only: update threshold when actual coverage is greater
    is_greater=$(awk -v a="$actual" -v c="$current" 'BEGIN {print (a > c)}')
    if [ "$is_greater" -eq 1 ]; then
        printf '%s\n' "$actual" > "$covgate_file"
        echo "⬆️  ${module_display}: ${current}% → ${actual}%"
        UPDATED=$((UPDATED + 1))
    else
        echo "─  ${module_display}: ${actual}% (threshold: ${current}%)"
        UNCHANGED=$((UNCHANGED + 1))
    fi
done < "$covgate_list"

echo ""
echo "Done. Updated: $UPDATED, Unchanged: $UNCHANGED"
