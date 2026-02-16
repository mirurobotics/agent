#!/bin/sh
set -e

git_repo_root_dir=$(git rev-parse --show-toplevel)
cd "$git_repo_root_dir"

SRC_DIR="agent/src"

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

# Get the minimum coverage threshold for a module (directory that contains .covgate)
# Use 0 in .covgate to skip coverage check entirely
get_threshold() {
    module_dir="$1"
    head -1 "${module_dir}/.covgate" | tr -d '[:space:]'
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
echo "Checking per-module coverage (modules discovered by .covgate under ${SRC_DIR})..."
echo ""

HAS_FAILURES=0

# Discover modules: every directory under SRC_DIR that contains a .covgate file
covgate_list=$(mktemp)
trap 'rm -f "$covgate_list"' EXIT
find "$SRC_DIR" -name '.covgate' -type f | sort > "$covgate_list"

while read -r covgate_file; do
    module_path=$(dirname "$covgate_file")
    module_display="${module_path#$SRC_DIR/}"
    module_dir_abs="$git_repo_root_dir/$module_path/"

    MIN=$(get_threshold "$module_path")

    if [ "$MIN" = "0" ]; then
        echo "⏭️  ${module_display}: skipped (threshold: 0)"
        continue
    fi

    coverage=$(echo "$COV_JSON" | jq -r --arg mod "$module_dir_abs" '
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
        echo "⚠️  ${module_display}: no source files found in coverage data"
        continue
    fi

    if [ "$(awk -v c="$coverage" -v m="$MIN" 'BEGIN {print (c >= m)}')" -eq 0 ]; then
        echo "❌ ${module_display}: ${coverage}% (requires ${MIN}%)"
        HAS_FAILURES=1
    else
        echo "✅ ${module_display}: ${coverage}% (requires ${MIN}%)"
    fi
done < "$covgate_list"

echo ""

if [ "$HAS_FAILURES" -eq 1 ]; then
    echo "ERROR: One or more modules are below their minimum coverage threshold"
    exit 1
fi

echo "✅ All modules meet minimum coverage requirement"
