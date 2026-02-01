#!/bin/sh
set -e 

# Set the target directory, use the git repo root if no argument provided
git_repo_root_dir=$(git rev-parse --show-toplevel)
TARGET_DIR="${1:-$git_repo_root_dir}"
cd "$TARGET_DIR"

command_exists() {
    command -v "$1" >/dev/null 2>&1
}

# Check if rustup is installed
if ! command_exists rustup; then
    echo "Installing Rustup"
    echo "-----------------"
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
    . "$HOME"/.cargo/env
fi

# update the rust toolchain
echo "Updating the Rust toolchain"
echo "---------------------------"
rustup update
echo ""

# Install the nightly toolchain if it's not installed
if ! rustup toolchain list | grep -q 'nightly'; then
    echo "Installing Rust nightly toolchain"
    echo "---------------------------------"
    rustup install nightly
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
if ! command_exists cargo-machete; then
    echo "Installing cargo-machete"
    echo "------------------------"
    cargo install cargo-machete
fi

# Check if cargo udeps is installed
if ! command_exists cargo-udeps; then
    echo "Installing cargo-udeps"
    echo "----------------------"
    cargo install cargo-udeps
fi

# Check if cargo diet is installed
if ! command_exists cargo-diet; then
    echo "Installing cargo-diet"
    echo "---------------------"
    cargo install cargo-diet
fi

# Check if cargo audit is installed
if ! command_exists cargo-audit; then
    echo "Installing cargo-audit"
    echo "----------------------"
    cargo install cargo-audit
fi

# ============================= PROJECT ROOT LINTING ================================ #

# update the cargo dependencies
echo "Updating the Cargo dependencies"
echo "-------------------------------"
cargo update --verbose
echo ""

# auto formats rust code (excluding generated libs)
echo "formatting code..."
cargo fmt --package miru-agent
echo ""

echo "Looking for unused external dependencies"
echo "----------------------------------------"
cargo machete
# cargo +nightly udeps
echo ""

echo "Looking for unused internal code"
echo "--------------------------------"
cargo diet 
echo ""

echo "Checking for security vulnerabilities"
echo "-------------------------------------"
cargo audit
echo ""

# rust's code quality linter (excluding generated libs)
echo "Running Clippy"
echo "--------------"
cargo clippy --package miru-agent --fix --allow-dirty
echo ""

echo "Linting complete"
echo ""