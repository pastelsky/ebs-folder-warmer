#!/bin/bash

# Test build script for rust-cache-warmer
# This builds and tests the project in a Linux Docker container

set -e

echo "🔧 Building rust-cache-warmer in Linux Docker container..."
echo ""

# Build the Docker image
echo "📦 Building Docker image..."
docker build -f Dockerfile.test -t rust-cache-warmer-test .

echo ""
echo "✅ Build completed successfully!"
echo ""

echo "🧪 Running additional tests..."
echo ""

# Test the binary directly
echo "📋 Testing binary directly:"
docker run --rm rust-cache-warmer-test

echo ""
echo "🚀 Testing with different options:"
docker run --rm rust-cache-warmer-test ./rust-cache-warmer/target/release/rust-cache-warmer --io-uring --libaio --direct-io /tmp/test-data

echo ""
echo "✅ All tests passed! The Linux build works correctly."
echo ""
echo "The binary is available at: ./rust-cache-warmer/target/release/rust-cache-warmer"
echo "" 