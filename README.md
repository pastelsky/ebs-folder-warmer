# Rust Cache Warmer

High-performance, concurrent file cache warmer for EBS volumes. Designed to efficiently warm Amazon EBS volumes from S3 snapshots.

## Features

- **Multiple I/O Strategies**: io_uring, libaio, OS hints (fadvise/madvise), Tokio async
- **Runtime Strategy Detection**: Automatically detects available features and falls back gracefully
- **Direct I/O Support**: Bypass OS page cache for pure EBS warming 
- **Sparse Reading**: Efficient sampling for large files
- **High Concurrency**: Configurable queue depths (32-512+ operations)
- **Cross-Platform**: Linux (io_uring/libaio), macOS (madvise), universal fallbacks

## Quick Start

```bash
# Standard usage (OS hints + fallbacks)
./rust-cache-warmer /path/to/files

# High performance (automatically detects availability)
./rust-cache-warmer --libaio --direct-io --queue-depth 256 /path/to/files

# Maximum performance (automatically detects availability)  
./rust-cache-warmer --io-uring --direct-io --queue-depth 512 /path/to/files
```

## Build

```bash
# Single build includes all features with runtime detection
cargo build --release
```

## Performance

| Strategy | Queue Depth | Throughput | Compatibility |
|----------|-------------|------------|---------------|
| OS Hints | 32 | 1x | Universal |
| Direct I/O | 128 | 1.5x | XFS/EBS |
| libaio | 256 | 3x | Linux 2.6+ |
| io_uring | 512+ | 5x | Linux 5.1+ |

*Note: The binary automatically detects available features and falls back gracefully.*

## CLI Options

```bash
Options:
  -q, --queue-depth <DEPTH>          Concurrent operations [default: 32]
  -T, --threads <THREADS>             File discovery threads [default: CPU cores]
      --sparse-large-files <SIZE>     Use sparse reading for files > SIZE bytes
      --max-file-size <SIZE>          Skip files larger than SIZE bytes
      --direct-io                     Use O_DIRECT (bypass OS cache)
      --libaio                        Request Linux AIO for high performance
      --io-uring                      Request io_uring for maximum performance
      --debug                         Detailed debug output
      --profile                       Generate flamegraph.svg profiling
```

## Strategy Selection

The warmer automatically selects the best available strategy:

1. **io_uring** (if requested and available) - Maximum performance on modern Linux
2. **libaio** (if requested and available) - High performance on Linux 
3. **OS hints** (fadvise/madvise) - Efficient on all platforms
4. **Tokio async** - Universal fallback

Strategy detection happens at startup and displays which features are available.

## Warming Strategy

1. **Triggers EBS fetch**: Any read operation causes EBS to fetch blocks from S3
2. **Avoids memory waste**: Direct I/O or immediate cache dropping prevents OS caching
3. **Efficient sampling**: Sparse reading for large files (64KB intervals)  
4. **High throughput**: Concurrent operations with appropriate queue depths