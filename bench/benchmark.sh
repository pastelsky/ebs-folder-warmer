#!/bin/bash
set -e

# Disk Warmer Benchmarking Suite
# Uses hyperfine for performance measurement and fio for disk workloads

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
DISK_WARMER="$PROJECT_ROOT/disk-warmer/disk-warmer"
RESULTS_DIR="$SCRIPT_DIR/results"
TEST_DIR="/tmp/disk-warmer-bench"
TEST_SIZE="1G"
DEVICE="${BENCHMARK_DEVICE:-}"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

log() {
    echo -e "${BLUE}[BENCH]${NC} $1"
}

warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

error() {
    echo -e "${RED}[ERROR]${NC} $1"
    exit 1
}

success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1"
}

check_dependencies() {
    log "Checking dependencies..."
    
    command -v hyperfine >/dev/null 2>&1 || error "hyperfine is required. Install with: sudo apt install hyperfine"
    command -v fio >/dev/null 2>&1 || error "fio is required. Install with: sudo apt install fio"
    
    if [ ! -f "$DISK_WARMER" ]; then
        log "Building disk-warmer..."
        cd "$PROJECT_ROOT/disk-warmer"
        make clean && make
        cd - >/dev/null
    fi
    
    if [ ! -x "$DISK_WARMER" ]; then
        error "disk-warmer binary not found at $DISK_WARMER"
    fi
}

setup_test_environment() {
    log "Setting up test environment..."
    
    # Create test directory
    sudo mkdir -p "$TEST_DIR"
    mkdir -p "$RESULTS_DIR"
    
    # Create test files with realistic content
    log "Creating test dataset (${TEST_SIZE})..."
    
    # Database-like files (random data)
    sudo mkdir -p "$TEST_DIR/db"
    sudo fio --name=create_db --directory="$TEST_DIR/db" --rw=write --bs=4k --size=512M --numjobs=1 --ioengine=sync --direct=1 >/dev/null 2>&1
    
    # Log files (sequential data)
    sudo mkdir -p "$TEST_DIR/logs"
    sudo dd if=/dev/zero of="$TEST_DIR/logs/app.log" bs=1M count=256 2>/dev/null
    sudo dd if=/dev/urandom of="$TEST_DIR/logs/debug.log" bs=1M count=128 2>/dev/null
    
    # Configuration files (small files)
    sudo mkdir -p "$TEST_DIR/config"
    for i in {1..100}; do
        sudo dd if=/dev/urandom of="$TEST_DIR/config/config_$i.conf" bs=4k count=1 2>/dev/null
    done
    
    # Web content (mixed sizes)
    sudo mkdir -p "$TEST_DIR/web"
    sudo fio --name=create_web --directory="$TEST_DIR/web" --rw=write --bs=64k --size=256M --numjobs=4 --ioengine=sync >/dev/null 2>&1
    
    sudo chown -R $(whoami):$(whoami) "$TEST_DIR" 2>/dev/null || true
    
    log "Test dataset created: $(du -sh $TEST_DIR | cut -f1)"
}

detect_device() {
    if [ -z "$DEVICE" ]; then
        # Try to auto-detect the device containing the test directory
        DEVICE=$(df "$TEST_DIR" | tail -1 | awk '{print $1}' | sed 's/[0-9]*$//')
        
        if [ -z "$DEVICE" ] || [ ! -b "$DEVICE" ]; then
            warn "Could not auto-detect block device. Some benchmarks will be skipped."
            warn "Set BENCHMARK_DEVICE=/dev/your-device to enable full benchmarks."
            return 1
        fi
    fi
    
    if [ ! -b "$DEVICE" ]; then
        error "Device $DEVICE is not a valid block device"
    fi
    
    log "Using device: $DEVICE"
    return 0
}

benchmark_directory_warming() {
    log "Benchmarking directory warming performance..."
    
    # Clear caches before each test
    clear_caches() {
        sync
        sudo sh -c 'echo 3 > /proc/sys/vm/drop_caches' 2>/dev/null || true
    }
    
    # Test different directory warming scenarios
    hyperfine \
        --warmup 1 \
        --min-runs 3 \
        --max-runs 5 \
        --prepare 'clear_caches' \
        --export-json "$RESULTS_DIR/directory_warming.json" \
        --export-markdown "$RESULTS_DIR/directory_warming.md" \
        "sudo $DISK_WARMER $TEST_DIR/db $DEVICE" \
        "sudo $DISK_WARMER $TEST_DIR/logs $DEVICE" \
        "sudo $DISK_WARMER $TEST_DIR/config $DEVICE" \
        "sudo $DISK_WARMER $TEST_DIR/web $DEVICE"
}

benchmark_full_disk_warming() {
    if ! detect_device; then
        warn "Skipping full disk warming benchmarks (no device detected)"
        return
    fi
    
    log "Benchmarking full disk warming vs directory-only..."
    
    clear_caches() {
        sync
        sudo sh -c 'echo 3 > /proc/sys/vm/drop_caches' 2>/dev/null || true
    }
    
    hyperfine \
        --warmup 1 \
        --min-runs 2 \
        --max-runs 3 \
        --prepare 'clear_caches' \
        --export-json "$RESULTS_DIR/full_vs_directory.json" \
        --export-markdown "$RESULTS_DIR/full_vs_directory.md" \
        --command-name "Directory Only" "sudo $DISK_WARMER $TEST_DIR $DEVICE" \
        --command-name "Full Disk" "sudo $DISK_WARMER --full-disk $TEST_DIR $DEVICE"
}

benchmark_warming_effectiveness() {
    log "Benchmarking warming effectiveness with fio workloads..."
    
    # Create fio job files
    cat > "$RESULTS_DIR/random_read.fio" << 'EOF'
[random_read]
directory=${TEST_DIR}/db
rw=randread
bs=4k
size=512M
numjobs=4
ioengine=libaio
iodepth=16
direct=1
time_based
runtime=30s
group_reporting
EOF

    cat > "$RESULTS_DIR/sequential_read.fio" << 'EOF'
[sequential_read]
directory=${TEST_DIR}/logs
rw=read
bs=1M
size=256M
numjobs=2
ioengine=libaio
iodepth=8
direct=1
time_based
runtime=30s
group_reporting
EOF

    clear_caches() {
        sync
        sudo sh -c 'echo 3 > /proc/sys/vm/drop_caches' 2>/dev/null || true
    }
    
    warm_directory() {
        if detect_device; then
            sudo "$DISK_WARMER" "$TEST_DIR" "$DEVICE" >/dev/null 2>&1
        fi
    }
    
    # Test random read performance
    log "Testing random read performance..."
    hyperfine \
        --warmup 1 \
        --min-runs 3 \
        --max-runs 5 \
        --prepare 'clear_caches' \
        --export-json "$RESULTS_DIR/random_read_cold.json" \
        --export-markdown "$RESULTS_DIR/random_read_cold.md" \
        --command-name "Cold Cache" "fio $RESULTS_DIR/random_read.fio --output-format=terse"
    
    hyperfine \
        --warmup 1 \
        --min-runs 3 \
        --max-runs 5 \
        --prepare 'clear_caches; warm_directory' \
        --export-json "$RESULTS_DIR/random_read_warm.json" \
        --export-markdown "$RESULTS_DIR/random_read_warm.md" \
        --command-name "After Warming" "fio $RESULTS_DIR/random_read.fio --output-format=terse"
    
    # Test sequential read performance  
    log "Testing sequential read performance..."
    hyperfine \
        --warmup 1 \
        --min-runs 3 \
        --max-runs 5 \
        --prepare 'clear_caches' \
        --export-json "$RESULTS_DIR/sequential_read_cold.json" \
        --export-markdown "$RESULTS_DIR/sequential_read_cold.md" \
        --command-name "Cold Cache" "fio $RESULTS_DIR/sequential_read.fio --output-format=terse"
        
    hyperfine \
        --warmup 1 \
        --min-runs 3 \
        --max-runs 5 \
        --prepare 'clear_caches; warm_directory' \
        --export-json "$RESULTS_DIR/sequential_read_warm.json" \
        --export-markdown "$RESULTS_DIR/sequential_read_warm.md" \
        --command-name "After Warming" "fio $RESULTS_DIR/sequential_read.fio --output-format=terse"
}

benchmark_configuration_options() {
    if ! detect_device; then
        warn "Skipping configuration benchmarks (no device detected)"
        return
    fi
    
    log "Benchmarking different configuration options..."
    
    clear_caches() {
        sync
        sudo sh -c 'echo 3 > /proc/sys/vm/drop_caches' 2>/dev/null || true
    }
    
    hyperfine \
        --warmup 1 \
        --min-runs 3 \
        --max-runs 5 \
        --prepare 'clear_caches' \
        --export-json "$RESULTS_DIR/configuration_options.json" \
        --export-markdown "$RESULTS_DIR/configuration_options.md" \
        --command-name "Default (4KB reads)" "sudo $DISK_WARMER $TEST_DIR $DEVICE" \
        --command-name "Large reads (64KB)" "sudo $DISK_WARMER --read-size-kb=64 $TEST_DIR $DEVICE" \
        --command-name "High queue depth" "sudo $DISK_WARMER --queue-depth=256 $TEST_DIR $DEVICE" \
        --command-name "Small stride" "sudo $DISK_WARMER --stride-kb=256 $TEST_DIR $DEVICE"
}

generate_report() {
    log "Generating benchmark report..."
    
    cat > "$RESULTS_DIR/benchmark_report.md" << EOF
# Disk Warmer Benchmark Report

Generated on: $(date)
Test environment: $(uname -a)
Device: ${DEVICE:-"Not detected"}

## Test Configuration

- Test directory: $TEST_DIR
- Dataset size: $(du -sh $TEST_DIR 2>/dev/null | cut -f1 || echo "Unknown")
- Disk warmer version: $($DISK_WARMER --version 2>/dev/null || echo "Unknown")

## Results Summary

EOF

    # Add results from each benchmark if they exist
    for file in "$RESULTS_DIR"/*.md; do
        if [ -f "$file" ] && [ "$(basename "$file")" != "benchmark_report.md" ]; then
            echo "### $(basename "$file" .md | tr '_' ' ' | sed 's/\b\w/\U&/g')" >> "$RESULTS_DIR/benchmark_report.md"
            echo "" >> "$RESULTS_DIR/benchmark_report.md"
            cat "$file" >> "$RESULTS_DIR/benchmark_report.md"
            echo "" >> "$RESULTS_DIR/benchmark_report.md"
        fi
    done
    
    success "Benchmark report generated: $RESULTS_DIR/benchmark_report.md"
}

cleanup() {
    log "Cleaning up test environment..."
    sudo rm -rf "$TEST_DIR" 2>/dev/null || true
}

show_usage() {
    cat << EOF
Disk Warmer Benchmarking Suite

Usage: $0 [OPTIONS] [BENCHMARKS...]

Options:
    -d, --device DEVICE     Block device to test against (e.g., /dev/nvme0n1)
    -s, --size SIZE         Test dataset size (default: 1G)
    -h, --help             Show this help message
    --skip-setup           Skip test environment setup
    --keep-data            Don't cleanup test data after benchmarks

Benchmarks:
    directory              Directory warming performance
    full-disk              Full disk vs directory-only comparison  
    effectiveness          Warming effectiveness with fio workloads
    configuration          Different configuration options
    all                    Run all benchmarks (default)

Environment Variables:
    BENCHMARK_DEVICE       Block device to use for testing
    TEST_DIR              Directory for test data (default: /tmp/disk-warmer-bench)

Examples:
    # Run all benchmarks with auto-detected device
    $0

    # Run specific benchmarks with custom device
    $0 -d /dev/nvme1n1 directory effectiveness
    
    # Run with larger dataset
    $0 -s 5G configuration

EOF
}

main() {
    local benchmarks=()
    local skip_setup=false
    local keep_data=false
    
    # Parse arguments
    while [[ $# -gt 0 ]]; do
        case $1 in
            -d|--device)
                DEVICE="$2"
                shift 2
                ;;
            -s|--size)
                TEST_SIZE="$2"
                shift 2
                ;;
            --skip-setup)
                skip_setup=true
                shift
                ;;
            --keep-data)
                keep_data=true
                shift
                ;;
            -h|--help)
                show_usage
                exit 0
                ;;
            directory|full-disk|effectiveness|configuration)
                benchmarks+=("$1")
                shift
                ;;
            all)
                benchmarks=(directory full-disk effectiveness configuration)
                shift
                ;;
            *)
                error "Unknown option: $1"
                ;;
        esac
    done
    
    # Default to all benchmarks if none specified
    if [ ${#benchmarks[@]} -eq 0 ]; then
        benchmarks=(directory full-disk effectiveness configuration)
    fi
    
    log "Starting disk warmer benchmarks..."
    log "Benchmarks to run: ${benchmarks[*]}"
    
    check_dependencies
    
    if [ "$skip_setup" = false ]; then
        setup_test_environment
    fi
    
    # Run selected benchmarks
    for benchmark in "${benchmarks[@]}"; do
        case $benchmark in
            directory)
                benchmark_directory_warming
                ;;
            full-disk)
                benchmark_full_disk_warming
                ;;
            effectiveness)
                benchmark_warming_effectiveness
                ;;
            configuration)
                benchmark_configuration_options
                ;;
        esac
    done
    
    generate_report
    
    if [ "$keep_data" = false ]; then
        cleanup
    else
        log "Test data preserved at: $TEST_DIR"
    fi
    
    success "Benchmarks completed! Results in: $RESULTS_DIR"
}

# Handle cleanup on exit
trap cleanup EXIT

main "$@" 