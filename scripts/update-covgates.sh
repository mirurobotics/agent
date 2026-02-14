#!/bin/sh
set -e

git_repo_root_dir=$(git rev-parse --show-toplevel)
cd "$git_repo_root_dir"

SRC_DIR="agent/src"
ABS_SRC_DIR="$git_repo_root_dir/$SRC_DIR"

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

echo "Running tests with coverage instrumentation..."
echo ""

# Run cargo-llvm-cov once, capture JSON output
set +e
COV_JSON=$(RUST_LOG=off cargo llvm-cov --json --package miru-agent --features test -- --test-threads=1)
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

UPDATED=0
UNCHANGED=0

# Iterate over each module directory
for module_path in "$SRC_DIR"/*/; do
    module=$(basename "$module_path")
    covgate_file="${module_path}.covgate"

    # Get current threshold
    current=""
    if [ -f "$covgate_file" ]; then
        current=$(head -1 "$covgate_file" | tr -d '[:space:]')
    fi

    # Use jq to aggregate coverage for files matching this module
    actual=$(echo "$COV_JSON" | jq -r --arg mod "$ABS_SRC_DIR/$module/" '
        [.data[0].files[] | select(.filename | startswith($mod))] |
        if length == 0 then "0.0"
        else
            (map(.summary.lines.count) | add) as $total |
            (map(.summary.lines.covered) | add) as $covered |
            if $total == 0 then "0.0"
            else (($covered / $total) * 10000 | floor) / 100 | tostring
            end
        end
    ')

    # If no .covgate file exists, create one with actual coverage
    if [ -z "$current" ]; then
        printf '%s\n' "$actual" > "$covgate_file"
        echo "✨ ${module}: created at ${actual}%"
        UPDATED=$((UPDATED + 1))
        continue
    fi

    # Skip modules with threshold 0 (opted out)
    if [ "$current" = "0" ]; then
        echo "⏭️  ${module}: skipped (threshold: 0)"
        UNCHANGED=$((UNCHANGED + 1))
        continue
    fi

    # Compare and update if actual > current
    is_greater=$(awk -v a="$actual" -v c="$current" 'BEGIN {print (a > c)}')
    if [ "$is_greater" -eq 1 ]; then
        printf '%s\n' "$actual" > "$covgate_file"
        echo "⬆️  ${module}: ${current}% → ${actual}%"
        UPDATED=$((UPDATED + 1))
    else
        echo "─  ${module}: ${actual}% (threshold: ${current}%)"
        UNCHANGED=$((UNCHANGED + 1))
    fi
done

# Handle standalone .rs files in src root ("root" pseudo-module)
covgate_file="$SRC_DIR/.covgate"
current=""
if [ -f "$covgate_file" ]; then
    current=$(head -1 "$covgate_file" | tr -d '[:space:]')
fi

actual=$(echo "$COV_JSON" | jq -r --arg dir "$ABS_SRC_DIR/" '
    [.data[0].files[] | select(
        (.filename | startswith($dir)) and
        (.filename | ltrimstr($dir) | contains("/") | not)
    )] |
    if length == 0 then "0.0"
    else
        (map(.summary.lines.count) | add) as $total |
        (map(.summary.lines.covered) | add) as $covered |
        if $total == 0 then "0.0"
        else (($covered / $total) * 10000 | floor) / 100 | tostring
        end
    end
')

if [ -z "$current" ]; then
    printf '%s\n' "$actual" > "$covgate_file"
    echo "✨ root: created at ${actual}%"
    UPDATED=$((UPDATED + 1))
elif [ "$current" = "0" ]; then
    echo "⏭️  root: skipped (threshold: 0)"
    UNCHANGED=$((UNCHANGED + 1))
else
    is_greater=$(awk -v a="$actual" -v c="$current" 'BEGIN {print (a > c)}')
    if [ "$is_greater" -eq 1 ]; then
        printf '%s\n' "$actual" > "$covgate_file"
        echo "⬆️  root: ${current}% → ${actual}%"
        UPDATED=$((UPDATED + 1))
    else
        echo "─  root: ${actual}% (threshold: ${current}%)"
        UNCHANGED=$((UNCHANGED + 1))
    fi
fi

echo ""
echo "Done. Updated: $UPDATED, Unchanged: $UNCHANGED"
