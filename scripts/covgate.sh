#!/bin/sh
set -e

git_repo_root_dir=$(git rev-parse --show-toplevel)
cd "$git_repo_root_dir"

DEFAULT_COVERAGE="${1:-80.0}"
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

# Get the minimum coverage threshold for a module
# Reads from .covgate file in the module directory if it exists
# Use 0 in .covgate to skip coverage check entirely
get_threshold() {
    module_dir="$1"
    covgate_file="${module_dir}/.covgate"
    if [ -f "$covgate_file" ]; then
        head -1 "$covgate_file" | tr -d '[:space:]'
    else
        echo "$DEFAULT_COVERAGE"
    fi
}

echo "Running tests with coverage instrumentation..."
echo ""

# Run cargo-llvm-cov once, capture JSON output
set +e
COV_JSON=$(RUST_LOG=off cargo llvm-cov --json --package miru-agent --features test -- --test-threads=1)
TEST_EXIT=$?
set -e

if [ "$TEST_EXIT" -ne 0 ]; then
    echo ""
    echo "ERROR: tests failed (exit $TEST_EXIT) — fix failing tests before checking coverage"
    exit 1
fi

echo ""
echo "Checking per-module coverage (default minimum: ${DEFAULT_COVERAGE}%)..."
echo ""

HAS_FAILURES=0

# Iterate over each module directory
for module_path in "$SRC_DIR"/*/; do
    module=$(basename "$module_path")

    # Get threshold
    MIN=$(get_threshold "$module_path")

    # Skip if threshold is 0
    if [ "$MIN" = "0" ]; then
        echo "⏭️  ${module}: skipped (threshold: 0)"
        continue
    fi

    # Use jq to aggregate coverage for files matching this module
    coverage=$(echo "$COV_JSON" | jq -r --arg mod "$ABS_SRC_DIR/$module/" '
        [.data[0].files[] | select(.filename | startswith($mod))] |
        if length == 0 then "no_files"
        else
            (map(.summary.lines.count) | add) as $total |
            (map(.summary.lines.covered) | add) as $covered |
            if $total == 0 then "0.0"
            else (($covered / $total) * 10000 | floor) / 100 | tostring
            end
        end
    ')

    if [ "$coverage" = "no_files" ]; then
        echo "⚠️  ${module}: no source files found in coverage data"
        continue
    fi

    # Compare against threshold
    if [ "$(awk -v c="$coverage" -v m="$MIN" 'BEGIN {print (c >= m)}')" -eq 0 ]; then
        echo "❌ ${module}: ${coverage}% (requires ${MIN}%)"
        HAS_FAILURES=1
    else
        echo "✅ ${module}: ${coverage}% (requires ${MIN}%)"
    fi
done

# Check standalone .rs files in src root ("root" pseudo-module)
MIN=$(get_threshold "$SRC_DIR")

if [ "$MIN" = "0" ]; then
    echo "⏭️  root: skipped (threshold: 0)"
else
    coverage=$(echo "$COV_JSON" | jq -r --arg dir "$ABS_SRC_DIR/" '
        [.data[0].files[] | select(
            (.filename | startswith($dir)) and
            (.filename | ltrimstr($dir) | contains("/") | not)
        )] |
        if length == 0 then "no_files"
        else
            (map(.summary.lines.count) | add) as $total |
            (map(.summary.lines.covered) | add) as $covered |
            if $total == 0 then "0.0"
            else (($covered / $total) * 10000 | floor) / 100 | tostring
            end
        end
    ')

    if [ "$coverage" = "no_files" ]; then
        echo "⚠️  root: no source files found in coverage data"
    elif [ "$(awk -v c="$coverage" -v m="$MIN" 'BEGIN {print (c >= m)}')" -eq 0 ]; then
        echo "❌ root: ${coverage}% (requires ${MIN}%)"
        HAS_FAILURES=1
    else
        echo "✅ root: ${coverage}% (requires ${MIN}%)"
    fi
fi

echo ""

if [ "$HAS_FAILURES" -eq 1 ]; then
    echo "ERROR: One or more modules are below their minimum coverage threshold"
    exit 1
fi

echo "✅ All modules meet minimum coverage requirement"
