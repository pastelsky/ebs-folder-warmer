# Disk Warmer Benchmarking Suite

Comprehensive performance benchmarking for the EBS Folder Warmer using `hyperfine` and `fio`.

## Overview

This benchmarking suite provides automated performance testing to:
- Measure disk warming effectiveness
- Compare different configuration options  
- Test various workload patterns
- Validate performance improvements

## Quick Start

```bash
# Run all benchmarks with auto-detected device
./benchmark.sh

# Run specific benchmarks with custom device
./benchmark.sh -d /dev/nvme1n1 directory effectiveness

# Run with larger dataset
./benchmark.sh -s 5G configuration
```

## Benchmark Types

### Directory Warming (`directory`)
Tests warming performance across different file types:
- **Database files**: Random data, 4K blocks
- **Log files**: Sequential data, large files
- **Config files**: Many small files
- **Web content**: Mixed-size files

### Full Disk Comparison (`full-disk`)
Compares directory-only vs full disk warming:
- Directory warming (default mode)
- Full disk warming (with `--full-disk` flag)

### Warming Effectiveness (`effectiveness`)
Measures actual performance improvement using FIO workloads:
- Random read performance (before/after warming)
- Sequential read performance (before/after warming)
- Real-world I/O patterns

### Configuration Options (`configuration`)
Tests different disk-warmer settings:
- Default settings (4KB reads, 512KB stride)
- Large reads (64KB blocks)
- High queue depth (256 concurrent operations)
- Small stride (256KB steps)

## FIO Workload Configurations

### Database Workload (`database-workload.fio`)
Simulates OLTP database patterns:
- 80% read, 20% write mix
- Random I/O patterns
- Multiple concurrent connections
- Transaction log simulation

### Web Server Workload (`web-server-workload.fio`)
Simulates web server file access:
- Static content serving
- Log file writes
- Cache updates
- Configuration file reads

### EBS Optimized (`ebs-optimized.fio`)
AWS EBS-specific patterns:
- GP3 baseline performance (3000 IOPS)
- IO2 high IOPS scenarios
- Throughput optimization
- Burst credit simulation
- Cold start performance

## Usage Examples

### Basic Performance Testing
```bash
# Quick benchmark for development
./benchmark.sh directory

# Full effectiveness testing
./benchmark.sh effectiveness

# Test all scenarios
./benchmark.sh all
```

### Advanced Configuration
```bash
# Use specific device and larger dataset
./benchmark.sh -d /dev/nvme2n1 -s 10G all

# Skip environment setup (reuse existing data)
./benchmark.sh --skip-setup effectiveness

# Keep test data for manual inspection
./benchmark.sh --keep-data configuration
```

### Environment Variables
```bash
# Set device via environment
export BENCHMARK_DEVICE=/dev/xvdf
./benchmark.sh

# Custom test directory
export TEST_DIR=/mnt/fast-ssd/test-data
./benchmark.sh
```

## Interpreting Results

### Timing Results (Hyperfine)
- **Mean**: Average execution time across runs
- **Min/Max**: Best and worst case performance
- **Standard Deviation**: Consistency of results

### FIO Results
- **IOPS**: Operations per second
- **Bandwidth**: Throughput in MB/s  
- **Latency**: Response time percentiles
- **CPU Usage**: System resource utilization

### What to Look For

**Good Performance Indicators:**
- Lower warming times for same dataset
- Higher IOPS after warming vs cold cache
- Consistent results across multiple runs
- Reasonable resource usage

**Performance Regression Signs:**
- Increased warming time
- Reduced post-warming performance
- High variance in results
- Excessive memory/CPU usage

## CI/CD Integration

### Automated Benchmarks
- **Pull Requests**: Quick directory and configuration benchmarks
- **Main Branch**: Effectiveness benchmarks  
- **Manual Trigger**: Full benchmark suite

### Benchmark Comparison
- Compares PR branch against main branch baseline
- Highlights performance improvements or regressions
- Posts results as PR comments

### Results Storage
- JSON and Markdown formats
- Uploaded as GitHub Actions artifacts
- 30-day retention for trend analysis

## Dependencies

### Required Tools
```bash
# Ubuntu/Debian
sudo apt install hyperfine fio build-essential libaio-dev

# CentOS/RHEL/Fedora  
sudo yum install hyperfine fio gcc make libaio-devel
```

### System Requirements
- **Disk Space**: 2-10GB for test datasets
- **Memory**: 4GB+ recommended
- **Permissions**: sudo access for cache clearing
- **Block Device**: For full testing (can auto-detect)

## Benchmark Configuration

### Test Dataset Sizes
- **Small**: 1GB (quick testing)
- **Medium**: 5GB (standard benchmarks)
- **Large**: 10GB+ (comprehensive testing)

### Runtime Settings
- **Warmup**: 1 iteration to stabilize
- **Min Runs**: 3 for statistical validity
- **Max Runs**: 5 to balance accuracy vs time
- **FIO Runtime**: 30-120s depending on test

## Troubleshooting

### Common Issues

**Permission Denied**
```bash
# Ensure sudo access for cache clearing
sudo echo "Testing sudo access"
```

**Device Not Found**
```bash
# List available devices
lsblk
sudo fdisk -l

# Set device manually
export BENCHMARK_DEVICE=/dev/your-device
```

**Insufficient Space**
```bash
# Check available space
df -h /tmp

# Use different location
export TEST_DIR=/path/to/larger/disk
```

**FIO Errors**
```bash
# Verify FIO installation
fio --version

# Check libaio availability
ls /usr/lib/x86_64-linux-gnu/libaio*
```

### Performance Tips

**For Accurate Results:**
- Run on dedicated hardware when possible
- Disable swap to avoid interference
- Clear caches between tests
- Use consistent test datasets

**For Faster Benchmarks:**
- Reduce dataset size with `-s 1G`
- Use fewer FIO runtime seconds
- Skip full disk tests on slow devices

## Contributing

### Adding New Benchmarks
1. Create new benchmark function in `benchmark.sh`
2. Add corresponding FIO configuration if needed
3. Update help text and documentation
4. Test with various devices and datasets

### Improving Accuracy
- Increase run counts for critical benchmarks
- Add more realistic workload patterns
- Include memory usage and CPU metrics
- Test on various storage types (SSD, HDD, NVMe)

## Example Output

```console
[BENCH] Starting disk warmer benchmarks...
[BENCH] Benchmarks to run: directory effectiveness
[BENCH] Using device: /dev/nvme1n1
[BENCH] Creating test dataset (5G)...
[BENCH] Benchmarking directory warming performance...

Benchmark 1: Database Files
  Time (mean ± σ):      2.341 s ±  0.123 s    [User: 0.045 s, System: 0.312 s]
  Range (min … max):    2.198 s …  2.534 s    5 runs
  
[BENCH] Testing random read performance...
[BENCH] Testing sequential read performance...
[SUCCESS] Benchmarks completed! Results in: bench/results
```

For more information, see the main project [README](../README.md). 