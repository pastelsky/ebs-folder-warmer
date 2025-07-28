use std::path::PathBuf;
use std::time::Instant;
use log::debug;

#[cfg(all(target_os = "linux", feature = "libaio"))]
use rio::{Rio, Completion};
#[cfg(target_os = "linux")]
use libc;

use crate::warming::{WarmingResult, WarmingOptions};

/// Warm file using Linux AIO (libaio) with optional direct I/O
#[cfg(all(target_os = "linux", feature = "libaio"))]
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

#[cfg(all(target_os = "linux", feature = "libaio"))]
async fn warm_with_libaio_direct(path: &PathBuf, file_size: u64, sparse_threshold: u64) -> Result<WarmingResult, std::io::Error> {
    let start = Instant::now();
    const ALIGNMENT: usize = 4096;
    const CHUNK_SIZE: usize = 1024 * 1024; // 1MB chunks
    const MAX_QUEUE_DEPTH: usize = 256; // High queue depth for better performance
    
    // Open file with O_DIRECT
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECT)
        .open(path)?;
    
    // Create Rio instance for async I/O
    let rio = Rio::new().map_err(|e| {
        debug!("Failed to create Rio instance: {}", e);
        std::io::Error::new(std::io::ErrorKind::Other, format!("Rio creation failed: {}", e))
    })?;
    
    if sparse_threshold > 0 && file_size > sparse_threshold {
        // Sparse reading with libaio for large files
        debug!("Using sparse libaio + direct I/O for large file ({} bytes)", file_size);
        let sample_interval: u64 = 65536; // 64KB intervals
        let mut samples_read = 0;
        
        // Calculate number of samples
        let num_samples = ((file_size + sample_interval - 1) / sample_interval) as usize;
        let batch_size = std::cmp::min(MAX_QUEUE_DEPTH, num_samples);
        
        // Allocate aligned buffers for direct I/O
        let mut buffers = Vec::new();
        let layout = std::alloc::Layout::from_size_align(ALIGNMENT, ALIGNMENT)
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::Other, "Failed to create aligned memory layout"))?;
        
        for _ in 0..batch_size {
            let buffer = unsafe { std::alloc::alloc(layout) };
            if buffer.is_null() {
                // Clean up allocated buffers
                for buf in buffers {
                    unsafe { std::alloc::dealloc(buf, layout) };
                }
                return Err(std::io::Error::new(std::io::ErrorKind::OutOfMemory, "Failed to allocate aligned buffer"));
            }
            buffers.push(buffer);
        }
        
        let result = async {
            let mut offset: u64 = 0;
            let mut batch_count = 0;
            
            while offset < file_size {
                let mut operations = Vec::new();
                
                // Submit a batch of reads
                for i in 0..batch_size {
                    if offset >= file_size { break; }
                    
                    let aligned_offset = (offset / ALIGNMENT as u64) * ALIGNMENT as u64;
                    let buffer_idx = i % buffers.len();
                    let buffer_slice = unsafe { 
                        std::slice::from_raw_parts_mut(buffers[buffer_idx], ALIGNMENT) 
                    };
                    
                    let completion = rio.read_at(&file, buffer_slice, aligned_offset);
                    operations.push(completion);
                    
                    offset += sample_interval;
                }
                
                // Wait for completions
                for completion in operations {
                    match completion.wait() {
                        Ok(bytes_read) => {
                            if bytes_read > 0 {
                                samples_read += 1;
                            }
                        }
                        Err(e) => {
                            debug!("libaio read failed: {}", e);
                        }
                    }
                }
                
                batch_count += 1;
            }
            Ok(())
        }.await;
        
        // Clean up buffers
        for buffer in buffers {
            unsafe { std::alloc::dealloc(buffer, layout) };
        }
        
        debug!("Sparse libaio + direct I/O completed: {} samples in {:?}", samples_read, start.elapsed());
        
        match result {
            Ok(()) => Ok(WarmingResult {
                method: "libaio_direct_sparse",
                success: true,
                duration: start.elapsed(),
            }),
            Err(e) => Err(e),
        }
    } else {
        // Full libaio + direct I/O reading for smaller files
        debug!("Using full libaio + direct I/O for file ({} bytes)", file_size);
        
        let num_chunks = ((file_size + CHUNK_SIZE as u64 - 1) / CHUNK_SIZE as u64) as usize;
        let batch_size = std::cmp::min(MAX_QUEUE_DEPTH, num_chunks);
        
        // Allocate aligned buffers
        let mut buffers = Vec::new();
        let layout = std::alloc::Layout::from_size_align(CHUNK_SIZE, ALIGNMENT)
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::Other, "Failed to create aligned memory layout"))?;
        
        for _ in 0..batch_size {
            let buffer = unsafe { std::alloc::alloc(layout) };
            if buffer.is_null() {
                for buf in buffers {
                    unsafe { std::alloc::dealloc(buf, layout) };
                }
                return Err(std::io::Error::new(std::io::ErrorKind::OutOfMemory, "Failed to allocate aligned buffer"));
            }
            buffers.push(buffer);
        }
        
        let result = async {
            let mut total_read = 0u64;
            let mut offset = 0u64;
            let mut batch_count = 0;
            
            while offset < file_size {
                let mut operations = Vec::new();
                
                // Submit a batch of reads
                for i in 0..batch_size {
                    if offset >= file_size { break; }
                    
                    let remaining = file_size - offset;
                    let read_size = std::cmp::min(CHUNK_SIZE as u64, remaining);
                    let aligned_read_size = ((read_size + ALIGNMENT as u64 - 1) / ALIGNMENT as u64) * ALIGNMENT as u64;
                    let actual_read_size = std::cmp::min(aligned_read_size, CHUNK_SIZE as u64) as usize;
                    
                    let buffer_idx = i % buffers.len();
                    let buffer_slice = unsafe { 
                        std::slice::from_raw_parts_mut(buffers[buffer_idx], actual_read_size) 
                    };
                    
                    let completion = rio.read_at(&file, buffer_slice, offset);
                    operations.push((completion, offset, actual_read_size));
                    
                    offset += actual_read_size as u64;
                }
                
                // Wait for completions
                for (completion, _read_offset, _size) in operations {
                    match completion.wait() {
                        Ok(bytes_read) => {
                            total_read += bytes_read as u64;
                        }
                        Err(e) => {
                            debug!("libaio read failed: {}", e);
                        }
                    }
                }
                
                batch_count += 1;
            }
            Ok(total_read)
        }.await;
        
        // Clean up buffers
        for buffer in buffers {
            unsafe { std::alloc::dealloc(buffer, layout) };
        }
        
        match result {
            Ok(bytes_read) => {
                debug!("Full libaio + direct I/O completed: {} bytes read in {:?}", bytes_read, start.elapsed());
                Ok(WarmingResult {
                    method: "libaio_direct_full",
                    success: true,
                    duration: start.elapsed(),
                })
            }
            Err(e) => Err(e),
        }
    }
}

// Stub implementation for when libaio feature is not enabled
#[cfg(not(all(target_os = "linux", feature = "libaio")))]
pub async fn warm_file(
    _path: &PathBuf,
    _file_size: u64,
    _options: &WarmingOptions,
) -> Result<WarmingResult, std::io::Error> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "libaio feature not enabled"
    ))
} 