#!/bin/bash

# A script to prepare the 'rust-cache-warmer' project for profiling.
# This script will:
# 1. Check for the Rust toolchain.
# 2. Install 'cargo-flamegraph' if it's not already installed.
# 3. Verify the Cargo.toml is configured for debug symbols.
# 4. Build the project in release mode.
# 5. Provide the command to run the profiler.

# Exit immediately if a command exits with a non-zero status.
set -e

# --- Helper Functions for Colored Output ---
info() {
    echo -e "\033[1;34m[INFO]\033[0m $1"
}

warn() {
    echo -e "\033[1;33m[WARN]\033[0m $1"
}

error() {
    echo -e "\033[1;31m[ERROR]\033[0m $1" >&2
    exit 1
}

# Navigate to the script's directory to run commands in the correct context.
cd "$(dirname "$0")"

# 1. CHECK FOR RUST TOOLCHAIN
info "Checking for Rust toolchain (cargo)..."
if ! command -v cargo &> /dev/null; then
    error "Rust and Cargo are not installed. Please install them to continue."
else
    info "Rust toolchain found."
fi

# 2. INSTALL FLAMEGRAPH
info "Checking for 'cargo-flamegraph'..."
if ! cargo flamegraph --version &> /dev/null; then
    warn "'cargo-flamegraph' is not installed. Installing now..."
    if cargo install flamegraph; then
        info "Successfully installed 'cargo-flamegraph'."
    else
        error "Failed to install 'cargo-flamegraph'. Please try installing it manually with 'cargo install flamegraph'."
    fi
else
    info "'cargo-flamegraph' is already installed."
fi

# 3. VERIFY CARGO.TOML CONFIGURATION
info "Verifying 'Cargo.toml' for debug symbols in release mode..."
if ! grep -q "\[profile.release\]" Cargo.toml || ! grep -q "debug = true" Cargo.toml; then
    warn "'Cargo.toml' is not configured to generate debug symbols for release builds."
    info "Please add the following lines to the end of 'rust-cache-warmer/Cargo.toml':"
    echo -e "\n[profile.release]\ndebug = true\n"
    error "Configuration check failed. Please update Cargo.toml and re-run this script."
else
    info "'Cargo.toml' is correctly configured."
fi

# 4. BUILD THE PROJECT
info "Building 'rust-cache-warmer' in release mode with debug symbols..."
if cargo build --release; then
    info "Build successful!"
else
    error "Cargo build failed. Please check the compilation errors above."
fi

# 5. PROVIDE PROFILING INSTRUCTIONS
BINARY_NAME="rust-cache-warmer"
echo
info "Setup complete! The project is ready for profiling."
info "To generate a flamegraph, run the following command from the repository root:"
warn "Note: This command requires 'sudo' because the profiler needs elevated permissions to inspect the running process."
echo
echo "  sudo cargo flamegraph --root --manifest-path rust-cache-warmer/Cargo.toml -- ."
echo 