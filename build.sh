#!/bin/bash
set -e

echo "=== Local Build Script for disk-warmer ==="

# Check if we're on Linux
if [[ "$OSTYPE" != "linux-gnu"* ]]; then
    echo "⚠️  This tool is designed for Linux and requires Linux-specific headers."
    echo "   Current OS: $OSTYPE"
    echo ""
    echo "   To build and test:"
    echo "   1. Use a Linux machine or VM"
    echo "   2. Use Docker: docker run --rm -v \$(pwd):/work -w /work ubuntu:latest bash -c 'apt update && apt install -y build-essential libaio-dev && ./build.sh'"
    echo "   3. Let GitHub Actions build it automatically on push"
    echo ""
    echo "   GitHub Actions will build for multiple architectures on every push to main."
    exit 0
fi

# Change to disk-warmer directory
cd disk-warmer

# Clean previous builds
echo "Cleaning previous builds..."
make clean || true

# Build standard version
echo "Building standard x86_64 version..."
make

# Verify build
echo "Verifying build..."
file disk-warmer
ldd disk-warmer

# Test basic functionality (help)
echo "Testing help output..."
./disk-warmer --help

echo ""
echo "✅ Build completed successfully!"
echo "Binary: $(pwd)/disk-warmer"
echo ""
echo "To test locally:"
echo "  sudo ./disk-warmer /path/to/directory /dev/your-device"
echo ""
echo "To cross-compile for ARM64 (requires cross-compilation tools):"
echo "  make arm64"
echo ""
echo "To build portable version:"
echo "  make portable" 