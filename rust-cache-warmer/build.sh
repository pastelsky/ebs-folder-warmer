#!/bin/bash

# A simple build and installation script for rust-cache-warmer.
# This script will check for the Rust toolchain, build the binary,
# and install it to /usr/local/bin.

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

# 1. CHECK FOR RUST TOOLCHAIN
info "Checking for Rust toolchain (cargo)..."
if ! command -v cargo &> /dev/null; then
    warn "Rust and Cargo are not installed."
    
    # Provide Ubuntu-specific instructions if possible
    if command -v apt-get &> /dev/null; then
        info "To install on Ubuntu, you can run:"
        info "sudo apt-get update && sudo apt-get install -y cargo"
    fi
    
    error "Please install Rust/Cargo to continue. The official method is to run: \n      curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
else
    info "Rust toolchain found."
fi

# 2. BUILD THE PROJECT
info "Building 'rust-cache-warmer' in release mode (this may take a moment)..."

# Navigate to the script's directory to ensure we can find the Cargo.toml file.
cd "$(dirname "$0")"

if cargo build --release; then
    info "Build successful!"
else
    error "Cargo build failed. Please check the compilation errors above."
fi

# 3. INSTALL THE BINARY
BINARY_NAME="rust-cache-warmer"
SOURCE_PATH="./target/release/$BINARY_NAME"
INSTALL_DIR="/usr/local/bin"

if [ ! -f "$SOURCE_PATH" ]; then
    error "Could not find the built binary at '$SOURCE_PATH'"
fi

info "Preparing to install '$BINARY_NAME' to '$INSTALL_DIR'..."
info "This may require administrator privileges (sudo) to write to the directory."

# Use sudo to move the binary to the installation directory.
if sudo mv "$SOURCE_PATH" "$INSTALL_DIR/$BINARY_NAME"; then
    info "Successfully installed '$BINARY_NAME' to '$INSTALL_DIR/$BINARY_NAME'"
else
    error "Failed to install with sudo. You may need to copy the binary manually:"
    info "sudo cp '$SOURCE_PATH' '$INSTALL_DIR/'"
fi

echo
info "Installation complete! You can now run 'rust-cache-warmer' from your terminal." 