#!/bin/sh
set -e

# Set the target directory, use the git repo root if no argument provided
git_repo_root_dir=$(git rev-parse --show-toplevel)
LINT_DIR="$git_repo_root_dir/tools/lint"

cd "$LINT_DIR"

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

# Check if cargo audit is installed
if ! command_exists cargo-audit; then
    echo "Installing cargo-audit"
    echo "----------------------"
    cargo install cargo-audit
fi

# ============================= LINT TOOL LINTING ================================ #

# update the cargo dependencies
echo "Updating the Cargo dependencies"
echo "-------------------------------"
cargo update --verbose
echo ""

# auto formats rust code
echo "Formatting Code..."
cargo fmt
echo ""

# check import grouping, ordering, and comment headers (dogfooding)
echo "Checking import formatting"
echo "--------------------------"
cargo run --manifest-path "$LINT_DIR/Cargo.toml" -- --path "$LINT_DIR/src" --fix --config "$LINT_DIR/.lint-imports.toml"
echo ""

echo "Looking for unused external dependencies"
echo "----------------------------------------"
cargo machete
echo ""

echo "Checking for security vulnerabilities"
echo "-------------------------------------"
cargo audit
echo ""

# rust's code quality linter
echo "Running Clippy (auto-fix)"
echo "-------------------------"
cargo clippy --fix --allow-dirty -- -D warnings
echo ""

echo "Running Clippy (check)"
echo "----------------------"
cargo clippy --no-deps -- -D warnings
echo ""

echo "Linting complete"
echo ""
