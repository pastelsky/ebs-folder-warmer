use std::path::PathBuf;
use std::time::Instant;
use log::debug;

#[cfg(target_os = "linux")]
use rio::{Rio, Completion};
#[cfg(target_os = "linux")]
use libc;

use crate::warming::{WarmingResult, WarmingOptions};

/// Warm file using Linux AIO (libaio) with optional direct I/O
#[cfg(target_os = "linux")]
pub async fn warm_file(
    path: &PathBuf,
    file_size: u64,
    options: &WarmingOptions,
) -> Result<WarmingResult, std::io::Error> {
    debug!("Using libaio + direct I/O for high-performance EBS warming: {}", path.display());
    
    if options.use_direct_io {
        warm_with_libaio_direct(path, file_size, options.sparse_large_files).await
    } else {
        // For now, if not using direct I/O, fall back to standard approach
        // Could implement buffered libaio in the future
        debug!("libaio without direct I/O not yet implemented, falling back");
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "libaio without direct I/O not yet implemented"
        ))
    }
}

#[cfg(target_os = "linux")]
async fn warm_with_libaio_direct(
    path: &PathBuf,
    file_size: u64,
    sparse_large_files: u64,
) -> Result<WarmingResult, std::io::Error> {
    let start = Instant::now();
    
    // Check if libaio (rio) is available by trying to create a Rio instance
    match Rio::new() {
        Ok(rio) => {
            // Use sparse reading for large files
            if sparse_large_files > 0 && file_size > sparse_large_files {
                warm_sparse_libaio_direct(&rio, path, file_size).await
            } else {
                warm_full_libaio_direct(&rio, path).await
            }
        }
        Err(_) => {
            // libaio not available
            Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "libaio not available on this system"
            ))
        }
    }
}

#[cfg(target_os = "linux")]
async fn warm_sparse_libaio_direct(
    rio: &Rio,
    path: &PathBuf,
    file_size: u64,
) -> Result<WarmingResult, std::io::Error> {
    let start = Instant::now();
    
    // Open file with O_DIRECT
    let fd = unsafe {
        libc::open(
            std::ffi::CString::new(path.to_string_lossy().as_ref()).unwrap().as_ptr(),
            libc::O_RDONLY | libc::O_DIRECT,
            0,
        )
    };
    
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    
    let block_size = 4096u64; // Standard block size
    let stride = 65536u64; // Read every 64KB
    let mut bytes_read = 0u64;
    
    // Allocate aligned buffer for direct I/O
    let layout = std::alloc::Layout::from_size_align(block_size as usize, block_size as usize)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::Other, "Failed to create aligned memory layout"))?;
    let buffer = unsafe { std::alloc::alloc(layout) };
    if buffer.is_null() {
        unsafe { libc::close(fd) };
        return Err(std::io::Error::new(std::io::ErrorKind::OutOfMemory, "Failed to allocate aligned buffer"));
    }
    
    let mut offset = 0;
    while offset < file_size {
        let buffer_slice = unsafe { std::slice::from_raw_parts_mut(buffer, block_size as usize) };
        
        match rio.read_at(&fd, buffer_slice, offset).await {
            Ok(n) if n > 0 => {
                bytes_read += n as u64;
            }
            Ok(_) => break, // EOF
            Err(e) => {
                debug!("libaio read error at offset {}: {}", offset, e);
                // Continue with next block on error
            }
        }
        
        offset += stride;
    }
    
    unsafe { 
        std::alloc::dealloc(buffer, layout);
        libc::close(fd);
    }
    
    debug!("Sparse libaio + direct I/O completed: {} bytes read in {:?}", bytes_read, start.elapsed());
    Ok(WarmingResult {
        method: "libaio_direct_sparse",
        success: true,
        duration: start.elapsed(),
    })
}

#[cfg(target_os = "linux")]
async fn warm_full_libaio_direct(
    rio: &Rio,
    path: &PathBuf,
) -> Result<WarmingResult, std::io::Error> {
    let start = Instant::now();
    
    // Open file with O_DIRECT
    let fd = unsafe {
        libc::open(
            std::ffi::CString::new(path.to_string_lossy().as_ref()).unwrap().as_ptr(),
            libc::O_RDONLY | libc::O_DIRECT,
            0,
        )
    };
    
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    
    let block_size = 65536; // 64KB blocks for efficient reading
    let mut total_bytes_read = 0u64;
    let mut offset = 0;
    
    // Allocate aligned buffer for direct I/O
    let layout = std::alloc::Layout::from_size_align(block_size, block_size)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::Other, "Failed to create aligned memory layout"))?;
    let buffer = unsafe { std::alloc::alloc(layout) };
    if buffer.is_null() {
        unsafe { libc::close(fd) };
        return Err(std::io::Error::new(std::io::ErrorKind::OutOfMemory, "Failed to allocate aligned buffer"));
    }
    
    loop {
        let buffer_slice = unsafe { std::slice::from_raw_parts_mut(buffer, block_size) };
        
        match rio.read_at(&fd, buffer_slice, offset as u64).await {
            Ok(n) if n > 0 => {
                total_bytes_read += n as u64;
                offset += n;
            }
            Ok(_) => break, // EOF
            Err(e) => {
                unsafe { 
                    std::alloc::dealloc(buffer, layout);
                    libc::close(fd);
                }
                return Err(e);
            }
        }
    }
    
    unsafe { 
        std::alloc::dealloc(buffer, layout);
        libc::close(fd);
    }
    
    debug!("Full libaio + direct I/O completed: {} bytes read in {:?}", total_bytes_read, start.elapsed());
    Ok(WarmingResult {
        method: "libaio_direct_full",
        success: true,
        duration: start.elapsed(),
    })
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
        "libaio only supported on Linux"
    ))
} 