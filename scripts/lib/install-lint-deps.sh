#!/bin/sh
# Shared lint dependency installer — idempotent, skips already-installed tools.
#
# Installs the common lint toolchain: rustup, rustfmt, clippy,
# cargo-machete, cargo-audit, cargo-diet.
# Per-crate lint scripts can install additional tools after sourcing this.
#
# Skips `rustup update` when CI=true (set automatically by GitHub Actions).
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
        cargo install "$tool"
    fi
}

# Check if rustup is installed
if ! command_exists rustup; then
    echo "Installing Rustup"
    echo "-----------------"
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
    . "$HOME"/.cargo/env
fi

# Update the rust toolchain (skip in CI — toolchain is pinned by GH Actions)
if [ "$CI" != "true" ]; then
    echo "Updating the Rust toolchain"
    echo "---------------------------"
    rustup update
    echo ""
fi

# Check if cargo fmt is installed (part of rustfmt)
if ! rustup component list --installed | grep -q 'rustfmt'; then
    echo "Installing rustfmt (cargo fmt)"
    echo "-----------------------------"
    rustup component add rustfmt
fi

# Check if cargo clippy is installed
if ! rustup component list --installed | grep -q 'clippy'; then
    echo "Installing clippy"
    echo "-----------------"
    rustup component add clippy
fi

# Check if cargo machete is installed
if ! command_exists cargo-machete && ! has_cargo_subcommand cargo-machete; then
    echo "Installing cargo-machete"
    echo "------------------------"
    install_cargo_tool cargo-machete
fi

# Check if cargo audit is installed
if ! command_exists cargo-audit && ! has_cargo_subcommand cargo-audit; then
    echo "Installing cargo-audit"
    echo "----------------------"
    install_cargo_tool cargo-audit
fi

# Check if cargo diet is installed
if ! command_exists cargo-diet && ! has_cargo_subcommand cargo-diet; then
    echo "Installing cargo-diet"
    echo "---------------------"
    install_cargo_tool cargo-diet
fi
