# EBS Folder Warmer

A high-performance disk warming utility that prioritizes specific directories before warming the entire disk. Optimized for AWS EBS volumes and other block devices.

## Overview

The disk warmer has two operating modes:

**Default Mode (Directory Only)**:
- Discovers and warms only files in your target directory for immediate performance benefits
- Fast and focused warming of critical application data

**Full Disk Mode** (with `--full-disk` flag):
- **Phase 1**: Discovers and warms files in your target directory first
- **Phase 2**: Sequentially warms the remaining disk blocks to ensure complete coverage

This approach lets you choose between targeted directory warming or comprehensive disk coverage.

## Features

- ✅ **Flexible modes**: Directory-only (default) or full disk warming (optional)
- ✅ **Smart prioritization**: Directory files warmed first when using full disk mode
- ✅ **Modern async I/O**: io_uring (Linux 5.1+) with automatic fallback to Linux AIO
- ✅ **Direct I/O**: O_DIRECT flag bypasses page cache for optimal performance
- ✅ **Automatic alignment**: Device sector size detection and alignment for direct I/O
- ✅ **Physical extent mapping**: Uses FIEMAP to read actual disk sectors
- ✅ **Smart deduplication**: Avoids re-reading blocks in full disk mode
- ✅ **Progress tracking**: Real-time progress with timing information
- ✅ **Flexible configuration**: Customizable read sizes, stride, and queue depth

## Installation

### Pre-built Binaries (Recommended)

Download pre-built binaries from [GitHub Releases](https://github.com/pastelsky/ebs-folder-warmer/releases):

```bash
# Download the latest release for your architecture
wget https://github.com/pastelsky/ebs-folder-warmer/releases/latest/download/disk-warmer-linux-x86_64.tar.gz

# Extract and install
tar -xzf disk-warmer-linux-x86_64.tar.gz
sudo ./disk-warmer-linux-x86_64/install.sh
```

**Available builds:**
- `disk-warmer-linux-x86_64.tar.gz` - Standard x86_64 build (most common)
- `disk-warmer-linux-x86_64-portable.tar.gz` - Portable x86_64 build (wide compatibility)
- `disk-warmer-linux-x86_64-static.tar.gz` - Static x86_64 build (maximum compatibility, no library dependencies)

> **Note**: ARM64/AArch64 builds will be added in a future release once cross-compilation is properly configured.

### Where Does `install.sh` Come From?

The `install.sh` script is not included in the source code. It is automatically generated and added to the release packages by our GitHub Actions CI/CD pipeline.

When you download a release, the `install.sh` script will be included in the tarball, ready for you to use.

### Development Builds

Latest development builds from the main branch are automatically available:
- [Development Release](https://github.com/pastelsky/ebs-folder-warmer/releases/tag/dev) (updated on every push to main)

### Build from Source

If you prefer to build from source:

#### Prerequisites

Install required development packages:

```bash
# Ubuntu/Debian
sudo apt update
sudo apt install -y build-essential libaio-dev liburing-dev

# CentOS/RHEL/Fedora (RHEL 8+, Fedora 30+)
sudo yum install -y gcc make libaio-devel liburing-devel
# or for newer versions:
sudo dnf install -y gcc make libaio-devel liburing-devel

# Note: liburing is optional but recommended for best performance on Linux 5.1+
# The tool will automatically fallback to libaio if liburing is not available
```

#### Quick Build

```bash
# Clone the repository
git clone https://github.com/pastelsky/ebs-folder-warmer.git
cd ebs-folder-warmer

# Build using the provided script
./build.sh

# Or build manually
cd disk-warmer
make
```

#### Build Variants

```bash
# Standard dynamic build
make

# Portable dynamic build
make portable

# Static build (maximum compatibility, no library dependencies)
make static

# Static build with all features (includes liburing if available)
make static-full

# For ARM64 (requires cross-compilation tools - coming soon)
# make arm64
```

## Usage

### Basic Usage

```bash
sudo ./disk-warmer [OPTIONS] <directory> <device>
```

### Parameters

- `<directory>`: Target directory to prioritize (e.g., `/var/lib/mysql`, `/opt/app/data`)
- `<device>`: Block device to warm (e.g., `/dev/nvme1n1`, `/dev/xvdf`)

### Options

| Option | Description | Default |
|--------|-------------|---------|
| `-r, --read-size-kb=SIZE` | Size of each read request in KB | 4 |
| `-s, --stride-kb=SIZE` | Distance between reads in KB | 512 |
| `-q, --queue-depth=NUM` | Number of concurrent AIO requests | 128 |
| `-f, --full-disk` | Warm entire disk after directory (two-phase mode) | disabled |
| `-m, --merge-extents` | Merge adjacent extents for larger sequential reads | disabled |
| `-l, --syslog` | Log output to syslog | disabled |
| `--silent` | Suppress progress output to stderr | disabled |
| `-h, --help` | Display help and exit | - |
| `-v, --version` | Output version information and exit | - |

### Examples

#### Basic Usage - Warm only MySQL data directory (default)
```bash
sudo ./disk-warmer /var/lib/mysql /dev/nvme1n1
```

#### Full disk warming with directory priority
```bash
sudo ./disk-warmer --full-disk /var/lib/mysql /dev/nvme1n1
```

#### High-throughput full disk warming with custom settings
```bash
sudo ./disk-warmer \
    --full-disk \
    --read-size-kb=64 \
    --stride-kb=256 \
    --queue-depth=256 \
    /opt/app/data /dev/xvdf
```

#### EBS-optimized warming with extent merging
```bash
sudo ./disk-warmer \
    --merge-extents \
    --read-size-kb=64 \
    --queue-depth=256 \
    /var/lib/mysql /dev/nvme1n1
```

#### Silent mode with syslog (directory only)
```bash
sudo ./disk-warmer --silent --syslog /home/ubuntu/important-data /dev/nvme2n1
```

### Sample Output

**Directory-only mode (default):**
```console
=== Phase 1: Discovering and warming directory files ===
Found 1,247 extents in directory to warm.
Directory extents sorted for sequential reading.
Phase 1 - Directory files: 1247 / 1247 (100%)
Phase 1 (directory warming) completed in 12.34 seconds

=== Directory warming completed successfully ===
Total warming time completed in 12.34 seconds
```

**Full disk mode (with --full-disk flag):**
```console
=== Phase 1: Discovering and warming directory files ===
Found 1,247 extents in directory to warm.
Directory extents sorted for sequential reading.
Phase 1 - Directory files: 1247 / 1247 (100%)
Phase 1 (directory warming) completed in 12.34 seconds

=== Phase 2: Warming remaining disk blocks ===
Phase 2 - Remaining disk: 98753 / 98753 (100%)
Phase 2 (remaining disk warming) completed in 145.67 seconds

=== Two-phase disk warming completed successfully ===
Total warming time completed in 158.01 seconds
```

## AWS EBS Usage

### Find Your EBS Device

```bash
# List block devices
lsblk

# Find the device name (commonly /dev/nvme1n1, /dev/xvdf, etc.)
sudo fdisk -l
```

### Common Use Cases

#### Web Application Cache Warming
```bash
# Warm only web application assets and cache (fast startup)
sudo ./disk-warmer /var/www/html /dev/nvme1n1

# Warm web assets first, then entire disk
sudo ./disk-warmer --full-disk /var/www/html /dev/nvme1n1
```

#### Database Warming
```bash
# MySQL/MariaDB (directory only - faster startup)
sudo ./disk-warmer /var/lib/mysql /dev/nvme1n1

# MySQL with extent merging for fragmented databases (EBS optimization)
sudo ./disk-warmer --merge-extents /var/lib/mysql /dev/nvme1n1

# MySQL with full disk warming
sudo ./disk-warmer --full-disk /var/lib/mysql /dev/nvme1n1

# PostgreSQL (directory only)
sudo ./disk-warmer /var/lib/postgresql /dev/nvme1n1

# MongoDB with extent merging (good for fragmented collections)
sudo ./disk-warmer --merge-extents /var/lib/mongodb /dev/nvme1n1
```

#### Application Data
```bash
# Custom application data (directory only)
sudo ./disk-warmer /opt/myapp/data /dev/nvme1n1

# Full disk warming prioritizing app data
sudo ./disk-warmer --full-disk /opt/myapp/data /dev/nvme1n1
```

## Performance Tuning

### For SSD/NVMe (like EBS gp3, io2)
```bash
# Directory only (faster)
sudo ./disk-warmer \
    --read-size-kb=64 \
    --stride-kb=256 \
    --queue-depth=256 \
    /your/directory /dev/nvme1n1

# Full disk mode
sudo ./disk-warmer \
    --full-disk \
    --read-size-kb=64 \
    --stride-kb=256 \
    --queue-depth=256 \
    /your/directory /dev/nvme1n1
```

### For Magnetic Storage (like EBS st1)
```bash
# Directory only
sudo ./disk-warmer \
    --read-size-kb=1024 \
    --stride-kb=1024 \
    --queue-depth=32 \
    /your/directory /dev/xvdf

# Full disk mode
sudo ./disk-warmer \
    --full-disk \
    --read-size-kb=1024 \
    --stride-kb=1024 \
    --queue-depth=32 \
    /your/directory /dev/xvdf
```

### Memory-Constrained Systems
```bash
# Directory only (minimal resources)
sudo ./disk-warmer \
    --read-size-kb=4 \
    --queue-depth=32 \
    /your/directory /dev/nvme1n1
```

## Performance Optimizations (v1.3.0+)

The latest version includes several performance enhancements based on recent Linux kernel advancements:

### io_uring Support (Linux 5.1+)
- **Automatic Detection**: Uses io_uring when available, falls back to Linux AIO
- **Performance Gain**: 20-50% faster on modern NVMe SSDs
- **Lower CPU Usage**: Reduced context switches via shared ring buffers
- **Scalability**: Better performance at high queue depths (256+)

### Direct I/O (O_DIRECT)
- **Bypass Page Cache**: Reduces memory pressure and improves raw performance
- **EBS Optimized**: Ideal for cloud storage where local cache isn't beneficial
- **Automatic Fallback**: Uses buffered I/O if O_DIRECT fails

### Automatic Device Alignment
- **Sector Detection**: Queries logical and physical sector sizes via ioctl
- **Auto-Alignment**: Adjusts read_size and stride for optimal direct I/O
- **4K Sector Support**: Properly handles modern SSDs with 4096-byte sectors

### Extent Merging (EBS-Specific Optimization)
- **Adjacent Extent Merging**: Combines consecutive file extents into larger reads
- **EBS-Aware Limits**: Caps merges at 16MB to respect S3 object boundaries
- **Reduced I/O Overhead**: Fewer requests to EBS, better for fragmented databases
- **Conditional Benefit**: Most effective with fragmented files (databases, VMs)

### Performance Tips
```bash
# For maximum performance on modern NVMe (Linux 5.1+)
sudo ./disk-warmer \
    --read-size-kb=64 \
    --stride-kb=256 \
    --queue-depth=256 \
    /your/directory /dev/nvme1n1

# Check what features are active
sudo ./disk-warmer --help  # Shows available features

# Verify build features
cd disk-warmer && make help  # Shows enabled performance features
```

### Expected Performance Improvements

| Feature | Improvement | Use Case |
|---------|-------------|----------|
| **io_uring vs libaio** | 20-50% faster | High queue depth (256+), NVMe SSDs |
| **O_DIRECT** | 10-30% faster | Large volumes, reduced memory usage |
| **Auto-alignment** | 5-15% improvement | Avoids sector misalignment penalties |
| **Extent merging** | 15-25% faster | Fragmented databases, many small files |
| **Combined** | **30-70% overall** | Modern NVMe with Linux 5.1+ |

*Benchmarks based on AWS EBS gp3/io2 volumes and modern NVMe SSDs*

### When to Use Extent Merging

**✅ Recommended for:**
- **Database servers** (MySQL, PostgreSQL) with fragmented tablespaces
- **Virtual machine images** stored as files
- **Large applications** with many small adjacent files
- **Cold EBS volumes** where S3 backend optimization matters

**❌ Not recommended for:**
- **Already sequential files** (videos, logs) - no adjacent extents to merge
- **Very small datasets** - merging overhead outweighs benefits  
- **Non-EBS storage** - optimization is EBS/S3-specific

**🔬 EBS-Specific Rationale:**
EBS volumes are backed by S3 objects (typically 16MB). Merging extents reduces the number of separate S3 requests but caps at 16MB to avoid crossing object boundaries, which could trigger unnecessary downloads of entire S3 objects.

## Permissions

The tool requires root permissions to:
- Open block devices for direct reading
- Access file extent mapping (FIEMAP)
- Perform asynchronous I/O operations

## Monitoring

### Using syslog
```bash
# Run with syslog enabled
sudo ./disk-warmer --syslog /var/lib/mysql /dev/nvme1n1

# Monitor progress in another terminal
sudo tail -f /var/log/syslog | grep disk-warmer
```

### Resource Monitoring
```bash
# Monitor I/O in another terminal
iostat -x 1

# Monitor system resources
htop
```

## Troubleshooting

### Common Issues

**Permission Denied**
```bash
# Ensure you're running as root
sudo ./disk-warmer /your/directory /dev/nvme1n1
```

**Device Not Found**
```bash
# Verify device exists
ls -la /dev/nvme1n1

# Check if it's mounted
mount | grep nvme1n1
```

**Compilation Errors**
```bash
# Install missing dependencies (Ubuntu/Debian)
sudo apt install -y build-essential libaio-dev liburing-dev

# For RHEL/CentOS/Fedora
sudo dnf install -y gcc make libaio-devel liburing-devel

# Check compiler version
gcc --version

# Check if liburing is available
pkg-config --exists liburing && echo "liburing available" || echo "liburing not found, will use libaio"
```

**Shared Library Errors (libaio.so.1t64, liburing.so.X)**
```bash
# If you see "error while loading shared libraries"
# Use the static build instead - no library dependencies
wget https://github.com/pastelsky/ebs-folder-warmer/releases/latest/download/disk-warmer-linux-x86_64-static.tar.gz
tar -xzf disk-warmer-linux-x86_64-static.tar.gz
sudo ./disk-warmer-linux-x86_64-static/install.sh

# Or build static version locally
cd disk-warmer
make static
sudo ./disk-warmer-static /your/directory /dev/your-device

# Check what libraries a binary depends on
ldd ./disk-warmer
# Static builds will show "not a dynamic executable"
```

**AIO Setup Failed**
```bash
# Check AIO limits
cat /proc/sys/fs/aio-max-nr

# Increase if needed (as root)
echo 1048576 > /proc/sys/fs/aio-max-nr
```

## Technical Details

- **io_uring**: Uses modern io_uring interface (Linux 5.1+) for high-performance async I/O with automatic fallback to Linux AIO
- **Direct I/O**: O_DIRECT flag bypasses page cache, reducing memory pressure and improving raw disk performance
- **Device Alignment**: Automatically detects and aligns to device sector sizes (512B, 4KB, etc.) for optimal direct I/O
- **FIEMAP**: Uses Linux FIEMAP ioctl to discover physical disk extents
- **Bitmap Tracking**: Efficiently tracks warmed blocks to avoid duplication
- **Sequential Optimization**: Sorts extents by physical location for optimal disk access

## Version History

- **v1.3.1**: Added extent merging optimization for EBS volumes (--merge-extents)
- **v1.3.0**: io_uring support, O_DIRECT I/O, automatic device alignment detection  
- **v1.2.0**: Two-phase warming with timing information
- **v1.1.0**: Single-phase warming with basic progress tracking
- **v1.0.0**: Initial release

## License

This project is open source. Please check the license file for details.

## Automated Releases

This project uses GitHub Actions for automated building and releasing:

### Release Types

**Tagged Releases** (stable):
- Created when you push a git tag starting with `v` (e.g., `v1.2.0`)
- Builds for multiple architectures (x86_64, ARM64, static)
- Creates a GitHub release with binaries attached
- Recommended for production use

**Development Releases** (latest):
- Automatically created on every push to the `main` branch
- Available at the `dev` tag
- Contains the latest features but may be unstable
- Good for testing new features

### Creating a Release

To create a new stable release:

```bash
# Tag the current commit
git tag v1.3.0
git push origin v1.3.0

# GitHub Actions will automatically:
# 1. Build binaries for all architectures
# 2. Create a GitHub release
# 3. Upload all binaries as release assets
```

### CI/CD Pipeline

The GitHub Actions workflow:
1. **Builds** on every push and pull request
2. **Creates** standard and portable x86_64 builds
3. **Packages** binaries with install scripts and documentation
4. **Publishes** releases automatically on tag pushes
5. **Updates** development releases on main branch pushes

### Build Matrix

| Build Type | Architecture | Compatibility | Use Case |
|------------|-------------|---------------|----------|
| Standard x86_64 | x86_64 | Modern Linux distros | General use |
| Portable x86_64 | x86_64 | Wide Linux compatibility | Older systems, broad compatibility |
| Static x86_64 | x86_64 | Maximum compatibility | Any Linux system, no library dependencies |

## Performance Benchmarking

This project includes a comprehensive benchmarking suite to measure and validate disk warming performance.

### Quick Benchmarks

```bash
# Run quick performance tests
cd bench
./benchmark.sh directory

# Test warming effectiveness with FIO workloads
./benchmark.sh effectiveness

# Compare configuration options
./benchmark.sh configuration
```

### Automated CI Benchmarks

- **Pull Requests**: Automatic performance testing with baseline comparison
- **Main Branch**: Effectiveness benchmarks to track performance trends
- **Manual Triggers**: Full benchmark suite for comprehensive testing

### Benchmark Features

- **Hyperfine Integration**: Precise timing measurements with statistical analysis
- **FIO Workloads**: Realistic database, web server, and EBS-optimized patterns
- **Before/After Comparison**: Measures actual warming effectiveness
- **CI/CD Integration**: Automated performance regression detection

For detailed benchmarking documentation, see [bench/README.md](bench/README.md).

## Static Analysis & Code Quality

This project uses essential static analysis tools to ensure code quality and performance.

### Quick Start

```bash
# Install analysis tools (Ubuntu/Debian)
sudo apt-get install -y cppcheck clang-tidy

# Run static analysis
make analyze
```

### Available Analysis Tools

| Tool | Purpose | Command |
|------|---------|---------|
| **cppcheck** | General static analysis | `make analyze-cppcheck` |
| **clang-tidy** | Linting and modernization | `make analyze-clang-tidy` |

### Sanitizer Builds

Build with runtime error detection:

```bash
# Build essential sanitizer variants
make sanitize-all

# Individual sanitizers  
make sanitize-address    # AddressSanitizer (memory errors)
make sanitize-undefined  # UndefinedBehaviorSanitizer
```

**Testing with sanitizers:**
```bash
# Build and test with AddressSanitizer
make sanitize-address
sudo ./disk-warmer-asan /tmp/test-dir /dev/loop0
```

### Analysis Reports

Static analysis generates detailed reports:

- `cppcheck-report.xml` - Machine-readable cppcheck results

### CI Integration

- **Every PR**: Automatic static analysis with results posted as comments
- **Every release**: Static analysis must pass before building
- **Artifacts**: Analysis reports uploaded for 30 days

### Development Best Practices

1. **Run analysis locally** before committing:
   ```bash
   make analyze
   ```

2. **Use sanitizers** for testing:
   ```bash
   make sanitize-address
   sudo ./disk-warmer-asan --help  # Test basic functionality
   ```

3. **Performance analysis**:
   ```bash
   make analyze-clang-tidy  # Performance suggestions
   ```

## Contributing

Contributions are welcome! Please feel free to submit issues and pull requests.

### Development Workflow

1. Fork the repository
2. Create a feature branch
3. Make your changes
4. **Run static analysis**: `make analyze`
5. **Test with sanitizers**: `make sanitize-all`
6. Test locally with `./build.sh`
7. Run benchmarks with `cd bench && ./benchmark.sh`
8. Submit a pull request
9. CI will automatically test, analyze, and benchmark your changes

### Code Quality Requirements

- All static analysis checks must pass
- Code must compile cleanly with sanitizers
- Follow existing code style (checked by clang-tidy)
