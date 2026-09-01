#!/bin/sh
set -e

# Simple build script for Jinja2 templates
# This script handles installation of dependencies and building

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

log() { echo "${GREEN}==>${NC} $1"; }
warn() { echo "${YELLOW}Warning:${NC} $1"; }
error() { echo "${RED}Error:${NC} $1"; }

# Create a virtual environment if it doesn't exist
if [ ! -d ".venv" ]; then
    log "Creating virtual environment..."
    python3 -m venv .venv
fi

. .venv/bin/activate

# Check if Python 3 is available
if ! command -v python3 >/dev/null 2>&1; then
    error "Python 3 is required but not installed"
    exit 1
fi

# Check if jinja2 is installed
if ! python3 -c "import jinja2" 2>/dev/null; then
    log "Installing jinja2..."
    if command -v pip3 >/dev/null 2>&1; then
        python3 -m pip install jinja2 pyyaml types-pyyaml
    else
        error "pip not found. Please install jinja2 manually: pip install jinja2 pyyaml"
        exit 1
    fi
fi

# Run the build
log "Building scripts from Jinja2 templates..."
cd "$SCRIPT_DIR"

python3 render.py --config install.yaml --output-dir ../install

log "Build complete!"
