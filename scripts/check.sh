#!/bin/sh
set -e
REPO_ROOT=$(git rev-parse --show-toplevel)


echo "Lint"
echo "===="
"$REPO_ROOT/scripts/lint.sh"

echo "Test"
echo "===="
"$REPO_ROOT/scripts/covgate.sh"