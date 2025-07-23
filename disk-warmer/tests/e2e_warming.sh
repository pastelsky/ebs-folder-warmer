#!/bin/bash
set -e

# Setup test disk
dd if=/dev/zero of=test.img bs=1M count=10
LOOP_DEV=$(losetup -f --show test.img) || { echo 'losetup failed'; exit 1; }
rm -rf test_dir test.img output.txt

# Create test dir
mkdir -p test_dir/subdir
echo 'test content' > test_dir/file1.txt
ln -s file1.txt test_dir/symlink.txt || true

# Test directory-only mode
../disk-warmer test_dir $LOOP_DEV > output.txt
if ! grep -q 'completed successfully' output.txt; then
    echo 'Directory mode test failed'
    exit 1
fi

# Test full-disk mode
../disk-warmer --full-disk test_dir $LOOP_DEV > output.txt
if ! grep -q 'Two-phase disk warming completed successfully' output.txt; then
    echo 'Full-disk mode test failed'
    exit 1
fi

# Cleanup
rm -rf test_dir test.img output.txt
losetup -d $LOOP_DEV || true
echo 'All tests passed!' 