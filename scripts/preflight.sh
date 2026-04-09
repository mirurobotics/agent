#!/usr/bin/env bash
set -euo pipefail
REPO_ROOT=$(git rev-parse --show-toplevel)

# Run a command with each output line prefixed by [label].
# Returns the wrapped command's exit code.
run_prefixed() {
	local label=$1; shift
	local exit_code=0
	"$@" 2>&1 | while IFS= read -r line; do
		printf '[%s] %s\n' "$label" "$line"
	done || exit_code="${PIPESTATUS[0]}"
	return "$exit_code"
}

kill_jobs() {
	# shellcheck disable=SC2046
	kill $(jobs -p) 2>/dev/null || true
	wait 2>/dev/null || true
}

run_prefixed "lint" "$REPO_ROOT/scripts/lint.sh" &
run_prefixed "test" "$REPO_ROOT/scripts/covgate.sh" &

# Exit as soon as either job fails; kill the other immediately.
wait -n || { kill_jobs; echo ""; echo "Preflight FAILED"; exit 1; }
wait -n || { echo ""; echo "Preflight FAILED"; exit 1; }

echo ""
echo "Preflight clean"
