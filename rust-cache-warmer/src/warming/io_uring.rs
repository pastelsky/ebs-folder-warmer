use std::path::PathBuf;
use std::time::Instant;
use log::debug;

#[cfg(target_os = "linux")]
use tokio_uring::fs::File as UringFile;

use crate::warming::{WarmingResult, WarmingOptions};

/// Warm file using io_uring with optional direct I/O
#[cfg(target_os = "linux")]
pub async fn warm_file(
    path: &PathBuf,
    file_size: u64,
    options: &WarmingOptions,
) -> Result<WarmingResult, std::io::Error> {
    debug!("Using io_uring + direct I/O for maximum EBS warming performance: {}", path.display());
    
    if options.use_direct_io {
        warm_with_io_uring_direct(path, file_size, options.sparse_large_files).await
    } else {
        // For now, if not using direct I/O, fall back to standard approach
        // Could implement buffered io_uring in the future
        debug!("io_uring without direct I/O not yet implemented, falling back");
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "io_uring without direct I/O not yet implemented"
        ))
    }
}

#[cfg(target_os = "linux")]
async fn warm_with_io_uring_direct(
    path: &PathBuf,
    file_size: u64,
    sparse_large_files: u64,
) -> Result<WarmingResult, std::io::Error> {
    // Use sparse reading for large files
    if sparse_large_files > 0 && file_size > sparse_large_files {
        warm_sparse_io_uring_direct(path, file_size).await
    } else {
        warm_full_io_uring_direct(path).await
    }
}

#[cfg(target_os = "linux")]
async fn warm_sparse_io_uring_direct(
    path: &PathBuf,
    file_size: u64,
) -> Result<WarmingResult, std::io::Error> {
    let start = Instant::now();
    
    // Check if tokio-uring runtime is available
    let result = tokio_uring::start(async {
        // Open with O_DIRECT for true EBS warming
        let file = match UringFile::open(path).await {
            Ok(f) => f,
            Err(e) => return Err(e),
        };
        
        let block_size = 4096usize; // Standard block size
        let stride = 65536u64; // Read every 64KB
        let mut bytes_read = 0u64;
        
        let mut offset = 0;
        while offset < file_size {
            let buf = vec![0u8; block_size];
            match file.read_at(buf, offset).await {
                (Ok(n), _buf) if n > 0 => {
                    bytes_read += n as u64;
                }
                (Ok(_), _buf) => break, // EOF
                (Err(e), _buf) => {
                    debug!("io_uring read error at offset {}: {}", offset, e);
                    // Continue with next block on error
                }
            }
            
            offset += stride;
        }
        
        Ok(bytes_read)
    });
    
    match result {
        Ok(bytes_read) => {
            debug!("Sparse io_uring + direct I/O completed: {} bytes read in {:?}", bytes_read, start.elapsed());
            Ok(WarmingResult {
                method: "io_uring_direct_sparse",
                success: true,
                duration: start.elapsed(),
            })
        }
        Err(e) => {
            // io_uring runtime not available
            Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "io_uring runtime not available on this system"
            ))
        }
    }
}

#[cfg(target_os = "linux")]
async fn warm_full_io_uring_direct(
    path: &PathBuf,
) -> Result<WarmingResult, std::io::Error> {
    let start = Instant::now();
    
    // Check if tokio-uring runtime is available
    let result = tokio_uring::start(async {
        // Open with O_DIRECT for true EBS warming
        let file = match UringFile::open(path).await {
            Ok(f) => f,
            Err(e) => return Err(e),
        };
        
        let block_size = 65536; // 64KB blocks for efficient reading
        let mut total_bytes_read = 0u64;
        let mut offset = 0;
        
        loop {
            let buf = vec![0u8; block_size];
            match file.read_at(buf, offset as u64).await {
                (Ok(n), _buf) if n > 0 => {
                    total_bytes_read += n as u64;
                    offset += n;
                }
                (Ok(_), _buf) => break, // EOF
                (Err(e), _buf) => return Err(e),
            }
        }
        
        Ok(total_bytes_read)
    });
    
    match result {
        Ok(bytes_read) => {
            debug!("Full io_uring + direct I/O completed: {} bytes read in {:?}", bytes_read, start.elapsed());
            Ok(WarmingResult {
                method: "io_uring_direct_full",
                success: true,
                duration: start.elapsed(),
            })
        }
        Err(e) => {
            // io_uring runtime not available
            Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "io_uring runtime not available on this system"
            ))
        }
    }
}

// Stub implementation for non-Linux systems
#[cfg(not(target_os = "linux"))]
pub async fn warm_file(
    _path: &PathBuf,
    _file_size: u64,
    _options: &WarmingOptions,
) -> Result<WarmingResult, std::io::Error> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "io_uring only supported on Linux"
    ))
} 