#!/bin/sh
# Shared dependency updater — called by per-crate wrapper scripts.
#
# Required env:
#   CRATE_DIR  — absolute path to the crate root
set -e

cd "$CRATE_DIR"

cargo update --verbose
echo ""
