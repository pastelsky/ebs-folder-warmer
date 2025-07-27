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
-   `--debug`: Enable detailed debug logging.
-   `--ignore-hidden`: Ignore hidden files and directories (files starting with '.').
-   `--max-file-size <BYTES>`: Skip files larger than this size in bytes (e.g., 1000000000 for 1GB). `0` means no limit.
-   `--sparse-large-files <BYTES>`: Use sparse reading for files larger than this size. `0` means disabled.
-   `--profile`: Enable profiling. When used, generates a `flamegraph.svg` in the current directory.

### Profiling

To generate a useful performance flamegraph, you must use the `profiling` binary from the releases and run it with the `--profile` flag.

```sh
# Using the profiling-enabled binary
./rust-cache-warmer-linux-amd64-profiling --profile /path/to/your/data
```
This will create a `flamegraph.svg` file that you can open in a web browser to analyze the application's performance.

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