#!/bin/sh
# Shared coverage dependency installer — idempotent.
#
# Installs tools required to run coverage checks:
# - llvm-tools-preview rustup component
# - cargo-llvm-cov
#
# Skips `rustup update` when CI=true (toolchain is pinned by GH Actions).
set -e

command_exists() {
    command -v "$1" >/dev/null 2>&1
}

has_cargo_subcommand() {
    cargo --list | awk -v cmd="$1" 'BEGIN { found = 0 } $1 == cmd { found = 1 } END { exit !found }'
}

install_cargo_tool() {
    tool="$1"
    if command_exists cargo-binstall; then
        cargo binstall --no-confirm --force "$tool"
    else
        cargo install --locked "$tool"
    fi
}

# Check if rustup is installed
if ! command_exists rustup; then
    echo "Installing Rustup"
    echo "-----------------"
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
    # shellcheck disable=SC1090
    . "$HOME"/.cargo/env
fi

# Update the rust toolchain (skip in CI — toolchain is pinned by GH Actions)
if [ "$CI" != "true" ]; then
    echo "Updating the Rust toolchain"
    echo "---------------------------"
    rustup update
    echo ""
fi

# Check if llvm-tools-preview is installed
if ! rustup component list --installed | grep -q 'llvm-tools-preview'; then
    echo "Installing llvm-tools-preview"
    echo "----------------------------"
    rustup component add llvm-tools-preview
    echo ""
fi

# Check if cargo-llvm-cov is installed
if ! command_exists cargo-llvm-cov && ! has_cargo_subcommand cargo-llvm-cov; then
    echo "Installing cargo-llvm-cov"
    echo "-------------------------"
    install_cargo_tool cargo-llvm-cov
    echo ""
fi
