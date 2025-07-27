# Rust Cache Warmer

A high-performance, concurrent file cache warmer written in Rust. This tool is ideal for preparing systems for performance-sensitive workloads by loading file data into the OS page cache.

## Features

-   **High Performance:** Uses a multi-threaded file discovery and an asynchronous, concurrent file reading engine powered by Tokio.
-   **Cross-Platform:** Works on Linux, macOS, and Windows (with OS-specific optimizations for Linux/macOS).
-   **Efficient Caching:**
    -   Uses OS-native hints (`posix_fadvise` on Linux, `madvise` on macOS) for zero-copy cache warming when possible.
    -   Supports a **sparse reading** mode for efficiently warming very large files on SSDs without reading the entire file.
-   **Flexible File Handling:**
    -   Adjustable concurrency level (`--queue-depth`).
    -   Follows symbolic links (`--follow-symlinks`).
    -   Can respect `.gitignore` files (`--respect-gitignore`).
    -   Can ignore hidden files (`--ignore-hidden`).
    -   Ability to skip files larger than a given size (`--max-file-size`).
-   **Profiling Support:**
    -   Built-in profiling support to generate a flamegraph for performance analysis.
    -   Separate profiling-enabled builds are provided in releases.

## Installation

You can download pre-compiled binaries from the [**GitHub Releases**](https://github.com/pastelsky/ebs-folder-warmer/releases) page.

Two binaries are provided for Linux:
1.  `rust-cache-warmer-linux-amd64`: The standard, stripped-down binary for production use.
2.  `rust-cache-warmer-linux-amd64-profiling`: A release build that includes debug symbols for performance profiling.

To install:
1.  Download the desired binary.
2.  Make it executable: `chmod +x ./rust-cache-warmer-linux-amd64`
3.  Move it to your system's PATH: `sudo mv ./rust-cache-warmer-linux-amd64 /usr/local/bin/rust-cache-warmer`

## Usage

```sh
rust-cache-warmer [OPTIONS] <DIRECTORIES>...
```

### Options

-   `-q, --queue-depth <QUEUE_DEPTH>`: Number of concurrent files to read (default: 128).
-   `-T, --threads <THREADS>`: Number of threads for file discovery.
-   `--follow-symlinks`: Follow symbolic links.
-   `--respect-gitignore`: Respect `.gitignore` and other ignore files.
-   `--max-depth <DEPTH>`: Maximum directory traversal depth.
-   `--debug`: Enable detailed debug logging with comprehensive performance metrics.
-   `--ignore-hidden`: Ignore hidden files and directories (files starting with '.').
-   `--max-file-size <BYTES>`: Skip files larger than this size in bytes (e.g., 1000000000 for 1GB). `0` means no limit.
-   `--sparse-large-files <BYTES>`: Use sparse reading for files larger than this size. `0` means disabled.
-   `--batch-size <SIZE>`: Number of files to process per async task batch (default: 1000). Higher values reduce coordination overhead for small files.
-   `--profile`: Enable profiling. When used, generates a `flamegraph.svg` in the current directory.

### Profiling

To generate a useful performance flamegraph, you must use the `profiling` binary from the releases and run it with the `--profile` flag.

```sh
# Using the profiling-enabled binary
./rust-cache-warmer-linux-amd64-profiling --profile /path/to/your/data
```
This will create a `flamegraph.svg` file that you can open in a web browser to analyze the application's performance.

### Debug Logging & Performance Analysis

The `--debug` flag enables comprehensive performance monitoring and logging:

- **Per-file timing**: File open, metadata fetch, and warming operation times
- **Concurrency analysis**: Semaphore wait times and queue efficiency metrics
- **I/O method tracking**: Which warming method is used (fadvise/madvise/fallback)
- **File size distribution**: Categorizes files as tiny/small/medium/large/huge
- **Throughput metrics**: MB/s, files/s, and concurrency efficiency percentages
- **Performance warnings**: Identifies slow operations for investigation

Example debug output:
```
DEBUG Processing medium file: /path/to/file.txt (65536 bytes)
DEBUG File open took 2.1ms for /path/to/file.txt
DEBUG fadvise operation took 0.3ms, success: true
DEBUG File /path/to/file.txt warming completed: method=linux_fadvise, duration=0.8ms, size=65536
DEBUG Performance metrics:
DEBUG   Throughput: 245.67 MB/s
DEBUG   Files per second: 1247.32
DEBUG   Concurrency efficiency: 78.4%
```

### Batch Processing Optimization

For workloads with many small files (like source code repositories), the `--batch-size` parameter can significantly improve performance by reducing async coordination overhead:

- **Small files (< 10KB)**: Use higher batch sizes (1000-5000) for better efficiency
- **Mixed file sizes**: Default batch size (1000) provides good balance
- **Large files (> 1MB)**: Lower batch sizes (100-500) prevent memory pressure

Example for source code repositories:
```sh
# Optimized for many small files (e.g., JavaScript/TypeScript projects)
rust-cache-warmer --batch-size 2000 --queue-depth 16 /path/to/source-code

# Balanced for mixed workloads
rust-cache-warmer --batch-size 1000 --queue-depth 32 /path/to/mixed-content
```

### Examples

**Basic Warming:**
Warm the cache for all files in a directory.
```sh
rust-cache-warmer /path/to/your/data
```

**Advanced Warming:**
Warm a large project directory, skipping files larger than 1GB and using sparse reads for files over 100MB.
```sh
rust-cache-warmer \
  --max-file-size 1000000000 \
  --sparse-large-files 100000000 \
  /mnt/my-large-project
``` 