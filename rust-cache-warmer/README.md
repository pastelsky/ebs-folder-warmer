# rust-cache-warmer

A high-performance, cross-platform utility to warm the operating system's page cache by reading files from one or more directories. Written in idiomatic Rust, it uses a multi-threaded and asynchronous approach to saturate I/O and warm the cache as quickly as possible.

This tool is ideal for preparing systems for performance-sensitive workloads, especially inside Docker containers or on systems where raw block device access is not available.

## Features

- **High Performance:** Uses a multi-threaded file discovery and an asynchronous, concurrent file reading engine powered by Tokio.
- **Cross-Platform:** Works on Linux, macOS, and Windows.
- **Docker Friendly:** Does not require elevated permissions or access to raw block devices.
- **Efficient:** Reads files in small chunks to warm the cache without consuming significant memory.
- **User-Friendly:** Provides clear progress bars and flexible command-line options.
- **Smart Discovery:** Can optionally respect `.gitignore` files and limit directory traversal depth.

## Installation

You can download the latest pre-compiled binary for Linux from the [**GitHub Releases**](https://github.com/pastelsky/ebs-folder-warmer/releases) page.

1.  **Download the Asset:**
    Go to the latest release and download the `rust-cache-warmer-linux-amd64` binary.

2.  **Make it Executable:**
    ```bash
    chmod +x ./rust-cache-warmer-linux-amd64
    ```

3.  **Move to Your PATH:**
    Move the binary to a directory in your system's `PATH` to make it accessible from anywhere.
    ```bash
    sudo mv ./rust-cache-warmer-linux-amd64 /usr/local/bin/rust-cache-warmer
    ```

4.  **Run it!**
    ```bash
    rust-cache-warmer --help
    ```

## Usage

Warm the cache for all files in the specified directories:
```bash
rust-cache-warmer /path/to/your/data /another/directory
```

For a full list of options, run:
```bash
rust-cache-warmer --help
``` 