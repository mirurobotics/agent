#!/usr/bin/env bash
set -euo pipefail
REPO_ROOT=$(git rev-parse --show-toplevel)

LINT_LOG=$(mktemp)
TEST_LOG=$(mktemp)
cleanup() { rm -f "$LINT_LOG" "$TEST_LOG"; }
trap cleanup EXIT

echo "Running lint..."
"$REPO_ROOT/scripts/lint.sh" >"$LINT_LOG" 2>&1 &
LINT_PID=$!

echo "Running tests..."
"$REPO_ROOT/scripts/covgate.sh" >"$TEST_LOG" 2>&1 &
TEST_PID=$!

LINT_EXIT=0; wait "$LINT_PID" || LINT_EXIT=$?
TEST_EXIT=0; wait "$TEST_PID" || TEST_EXIT=$?

echo ""
echo "=== Lint ==="
cat "$LINT_LOG"

echo ""
echo "=== Tests ==="
cat "$TEST_LOG"

if [ "$LINT_EXIT" -ne 0 ] || [ "$TEST_EXIT" -ne 0 ]; then
	echo ""
	echo "Preflight FAILED (lint=$LINT_EXIT tests=$TEST_EXIT)"
	exit 1
fi

echo ""
echo "Preflight clean"
