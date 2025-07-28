use std::path::PathBuf;
use std::time::Instant;
use log::debug;

#[cfg(all(target_os = "linux", feature = "io_uring"))]
use tokio_uring::fs::File as UringFile;
#[cfg(target_os = "linux")]
use libc;

use crate::warming::{WarmingResult, WarmingOptions};

/// Warm file using io_uring with optional direct I/O
#[cfg(all(target_os = "linux", feature = "io_uring"))]
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

#[cfg(all(target_os = "linux", feature = "io_uring"))]
async fn warm_with_io_uring_direct(path: &PathBuf, file_size: u64, sparse_threshold: u64) -> Result<WarmingResult, std::io::Error> {
    let start = Instant::now();
    const ALIGNMENT: usize = 4096;
    const CHUNK_SIZE: usize = 1024 * 1024; // 1MB chunks
    
    // Open file with O_DIRECT using tokio-uring
    let file = match tokio_uring::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECT)
        .open(path)
        .await {
            Ok(f) => f,
            Err(e) => {
                debug!("Failed to open file with io_uring + direct I/O: {}", e);
                return Err(e);
            }
        };
    
    if sparse_threshold > 0 && file_size > sparse_threshold {
        // Sparse reading with io_uring for large files
        debug!("Using sparse io_uring + direct I/O for large file ({} bytes)", file_size);
        let sample_interval: u64 = 65536; // 64KB intervals
        let mut offset: u64 = 0;
        let mut samples_read = 0;
        
        // Allocate aligned buffer for direct I/O
        let layout = std::alloc::Layout::from_size_align(ALIGNMENT, ALIGNMENT)
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::Other, "Failed to create aligned memory layout"))?;
        let buffer = unsafe { std::alloc::alloc(layout) };
        if buffer.is_null() {
            return Err(std::io::Error::new(std::io::ErrorKind::OutOfMemory, "Failed to allocate aligned buffer"));
        }
        
        let result = async {
            while offset < file_size {
                let aligned_offset = (offset / ALIGNMENT as u64) * ALIGNMENT as u64;
                let buffer_slice = unsafe { std::slice::from_raw_parts_mut(buffer, ALIGNMENT) };
                
                // Use io_uring for async read at specific offset
                match file.read_at(buffer_slice, aligned_offset).await {
                    Ok((res, _buf)) => {
                        if res == 0 { break; }
                        samples_read += 1;
                    }
                    Err(e) => {
                        debug!("io_uring read failed at offset {}: {}", aligned_offset, e);
                        break;
                    }
                }
                offset += sample_interval;
            }
            Ok(())
        }.await;
        
        unsafe { std::alloc::dealloc(buffer, layout) };
        debug!("Sparse io_uring + direct I/O completed: {} samples in {:?}", samples_read, start.elapsed());
        
        match result {
            Ok(()) => Ok(WarmingResult {
                method: "io_uring_direct_sparse",
                success: true,
                duration: start.elapsed(),
            }),
            Err(e) => Err(e),
        }
    } else {
        // Full io_uring + direct I/O reading for smaller files
        debug!("Using full io_uring + direct I/O for file ({} bytes)", file_size);
        
        let layout = std::alloc::Layout::from_size_align(CHUNK_SIZE, ALIGNMENT)
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::Other, "Failed to create aligned memory layout"))?;
        let buffer = unsafe { std::alloc::alloc(layout) };
        if buffer.is_null() {
            return Err(std::io::Error::new(std::io::ErrorKind::OutOfMemory, "Failed to allocate aligned buffer"));
        }
        
        let result = async {
            let mut total_read = 0u64;
            let mut offset = 0u64;
            
            while offset < file_size {
                let remaining = file_size - offset;
                let read_size = std::cmp::min(CHUNK_SIZE as u64, remaining);
                let aligned_read_size = ((read_size + ALIGNMENT as u64 - 1) / ALIGNMENT as u64) * ALIGNMENT as u64;
                let actual_read_size = std::cmp::min(aligned_read_size, CHUNK_SIZE as u64) as usize;
                
                let buffer_slice = unsafe { std::slice::from_raw_parts_mut(buffer, actual_read_size) };
                
                match file.read_at(buffer_slice, offset).await {
                    Ok((n, _buf)) => {
                        if n == 0 { break; }
                        total_read += n as u64;
                        offset += n as u64;
                    }
                    Err(e) => {
                        debug!("io_uring read failed at offset {}: {}", offset, e);
                        break;
                    }
                }
            }
            Ok(total_read)
        }.await;
        
        unsafe { std::alloc::dealloc(buffer, layout) };
        
        match result {
            Ok(bytes_read) => {
                debug!("Full io_uring + direct I/O completed: {} bytes read in {:?}", bytes_read, start.elapsed());
                Ok(WarmingResult {
                    method: "io_uring_direct_full",
                    success: true,
                    duration: start.elapsed(),
                })
            }
            Err(e) => Err(e),
        }
    }
}

// Stub implementation for when io_uring feature is not enabled
#[cfg(not(all(target_os = "linux", feature = "io_uring")))]
pub async fn warm_file(
    _path: &PathBuf,
    _file_size: u64,
    _options: &WarmingOptions,
) -> Result<WarmingResult, std::io::Error> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "io_uring feature not enabled"
    ))
} 