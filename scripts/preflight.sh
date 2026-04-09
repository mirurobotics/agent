#!/bin/sh
set -e
REPO_ROOT=$(git rev-parse --show-toplevel)

LINT_LOG=$(mktemp)
TEST_LOG=$(mktemp)

cleanup() {
	rm -f "$LINT_LOG" "$TEST_LOG"
}
trap cleanup EXIT

# Run lint and tests in parallel
"$REPO_ROOT/scripts/lint.sh" >"$LINT_LOG" 2>&1 &
LINT_PID=$!

"$REPO_ROOT/scripts/covgate.sh" >"$TEST_LOG" 2>&1 &
TEST_PID=$!

LINT_EXIT=0
TEST_EXIT=0
wait "$LINT_PID" || LINT_EXIT=$?
wait "$TEST_PID" || TEST_EXIT=$?

echo "Lint"
echo "===="
cat "$LINT_LOG"
echo ""

echo "Tests"
echo "====="
cat "$TEST_LOG"
echo ""

if [ "$LINT_EXIT" -ne 0 ] || [ "$TEST_EXIT" -ne 0 ]; then
	echo "Preflight FAILED (lint=$LINT_EXIT tests=$TEST_EXIT)"
	exit 1
fi

echo "Preflight clean"
