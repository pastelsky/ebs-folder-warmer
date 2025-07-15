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
- ✅ **Asynchronous I/O**: High-performance concurrent reads using Linux AIO
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

> **Note**: ARM64/AArch64 builds will be added in a future release once cross-compilation is properly configured.

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
sudo apt install -y build-essential libaio-dev

# CentOS/RHEL/Fedora
sudo yum install -y gcc make libaio-devel
# or for newer versions:
sudo dnf install -y gcc make libaio-devel
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

#### Cross-Compilation

```bash
# For ARM64 (requires cross-compilation tools)
make arm64

# For static linking (requires musl)
make static
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

# MySQL with full disk warming
sudo ./disk-warmer --full-disk /var/lib/mysql /dev/nvme1n1

# PostgreSQL (directory only)
sudo ./disk-warmer /var/lib/postgresql /dev/nvme1n1

# MongoDB (directory only)
sudo ./disk-warmer /var/lib/mongodb /dev/nvme1n1
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
# Install missing dependencies
sudo apt install -y build-essential libaio-dev

# Check compiler version
gcc --version
```

**AIO Setup Failed**
```bash
# Check AIO limits
cat /proc/sys/fs/aio-max-nr

# Increase if needed (as root)
echo 1048576 > /proc/sys/fs/aio-max-nr
```

## Technical Details

- **FIEMAP**: Uses Linux FIEMAP ioctl to discover physical disk extents
- **Linux AIO**: Leverages asynchronous I/O for maximum throughput
- **Bitmap Tracking**: Efficiently tracks warmed blocks to avoid duplication
- **Sequential Optimization**: Sorts extents by physical location for optimal disk access

## Version History

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

## Contributing

Contributions are welcome! Please feel free to submit issues and pull requests.

### Development Workflow

1. Fork the repository
2. Create a feature branch
3. Make your changes
4. Test locally with `./build.sh`
5. Submit a pull request
6. CI will automatically test your changes
