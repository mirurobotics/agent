#!/usr/bin/env bash
set -euo pipefail
REPO_ROOT=$(git rev-parse --show-toplevel)

LINT_LOG=$(mktemp)
TEST_LOG=$(mktemp)
TOOLS_LINT_LOG=$(mktemp)
TOOLS_TEST_LOG=$(mktemp)
cleanup() { rm -f "$LINT_LOG" "$TEST_LOG" "$TOOLS_LINT_LOG" "$TOOLS_TEST_LOG"; }
trap cleanup EXIT

echo "Running lint..."
"$REPO_ROOT/scripts/lint.sh" >"$LINT_LOG" 2>&1 &
LINT_PID=$!

echo "Running tests..."
"$REPO_ROOT/scripts/covgate.sh" >"$TEST_LOG" 2>&1 &
TEST_PID=$!

echo "Running tools lint..."
LINT_FIX=0 "$REPO_ROOT/tools/lint/scripts/lint.sh" >"$TOOLS_LINT_LOG" 2>&1 &
TOOLS_LINT_PID=$!

echo "Running tools tests..."
"$REPO_ROOT/tools/lint/scripts/covgate.sh" >"$TOOLS_TEST_LOG" 2>&1 &
TOOLS_TEST_PID=$!

LINT_EXIT=0; wait "$LINT_PID" || LINT_EXIT=$?
TEST_EXIT=0; wait "$TEST_PID" || TEST_EXIT=$?
TOOLS_LINT_EXIT=0; wait "$TOOLS_LINT_PID" || TOOLS_LINT_EXIT=$?
TOOLS_TEST_EXIT=0; wait "$TOOLS_TEST_PID" || TOOLS_TEST_EXIT=$?

echo ""
echo "=== Lint ==="
cat "$LINT_LOG"

echo ""
echo "=== Tests ==="
cat "$TEST_LOG"

echo ""
echo "=== Tools Lint ==="
cat "$TOOLS_LINT_LOG"

echo ""
echo "=== Tools Tests ==="
cat "$TOOLS_TEST_LOG"

if [ "$LINT_EXIT" -ne 0 ] || [ "$TEST_EXIT" -ne 0 ] || [ "$TOOLS_LINT_EXIT" -ne 0 ] || [ "$TOOLS_TEST_EXIT" -ne 0 ]; then
	echo ""
	echo "Preflight FAILED (lint=$LINT_EXIT tests=$TEST_EXIT tools_lint=$TOOLS_LINT_EXIT tools_tests=$TOOLS_TEST_EXIT)"
	exit 1
fi

echo ""
echo "Preflight clean"
