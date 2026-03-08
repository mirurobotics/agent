#!/bin/sh
set -e
REPO_ROOT=$(git rev-parse --show-toplevel)


echo "Lint"
echo "===="
"$REPO_ROOT/tools/lint/scripts/lint.sh"

echo "Test"
echo "===="
"$REPO_ROOT/tools/lint/scripts/covgate.sh"