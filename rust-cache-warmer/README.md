# Rust Cache Warmer

A high-performance, concurrent file cache warmer written in Rust.

## Features
- Concurrent file reading with adjustable queue depth
- Multi-threaded directory traversal
- Optional respect for .gitignore files
- Follow symbolic links
- Maximum traversal depth
- Debug logging
- Profiling with flamegraph
- Ignore hidden files (optional)
- Skip files larger than a specified size
- Sparse reading for large files to efficiently warm cache
- OS-specific cache advice for faster warming on Linux and macOS

## Usage

```sh
rust-cache-warmer [OPTIONS] <DIRECTORIES>...
```

### Options
- `-q, --queue-depth <QUEUE_DEPTH>`: Number of concurrent files to read (default: 128)
- `-T, --threads <THREADS>`: Number of threads for file discovery
- `--follow-symlinks`: Follow symbolic links
- `--respect-gitignore`: Respect .gitignore files
- `--max-depth <DEPTH>`: Maximum directory traversal depth
- `--debug`: Enable debug logging
- `--profile`: Enable profiling and generate flamegraph.svg
- `--ignore-hidden`: Ignore hidden files and directories
- `--max-file-size <BYTES>`: Skip files larger than this size (0 = no limit)
- `--sparse-large-files <BYTES>`: Use sparse reading for files larger than this size (0 = disabled)

### Examples
Warm cache for a directory:
```sh
rust-cache-warmer /path/to/dir
```

Skip files > 1GB and use sparse read for files > 100MB:
```sh
rust-cache-warmer --max-file-size 1000000000 --sparse-large-files 100000000 /path/to/dir
``` 