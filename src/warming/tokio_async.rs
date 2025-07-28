use std::path::PathBuf;
use std::time::Instant;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt, BufReader};
use log::debug;


#[cfg(target_os = "linux")]
use std::os::unix::fs::OpenOptionsExt;
#[cfg(target_os = "linux")]
use nix::fcntl::{posix_fadvise, PosixFadviseAdvice};
#[cfg(target_os = "linux")]
use libc;

use crate::warming::{WarmingResult, WarmingOptions};

/// Warm file using standard Tokio async I/O (with optional direct I/O)
pub async fn warm_file(
    path: &PathBuf,
    file_size: u64,
    options: &WarmingOptions,
) -> Result<WarmingResult, std::io::Error> {
    let _start = Instant::now();
    
    if options.use_direct_io && cfg!(target_os = "linux") {
        #[cfg(target_os = "linux")]
        {
            debug!("Using Tokio + direct I/O for {}", path.display());
            return warm_with_direct_io(path, file_size, options.sparse_large_files).await;
        }
    }
    
    // Standard Tokio async I/O with manual reading
    debug!("Using standard Tokio async I/O for {}", path.display());
    warm_with_manual_reading(path, file_size, options.sparse_large_files).await
}

#[cfg(target_os = "linux")]
async fn open_file_direct_io(path: &PathBuf) -> Result<File, std::io::Error> {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECT)
        .open(path)?;
    Ok(File::from_std(file))
}

#[cfg(target_os = "linux")]
async fn warm_with_direct_io(
    path: &PathBuf,
    file_size: u64,
    sparse_threshold: u64,
) -> Result<WarmingResult, std::io::Error> {
    let _start = Instant::now();
    const ALIGNMENT: usize = 4096; // 4KB alignment required for O_DIRECT
    const CHUNK_SIZE: usize = 1024 * 1024; // 1MB chunks for good throughput
    
    let mut file = open_file_direct_io(path).await?;
    
    if sparse_threshold > 0 && file_size > sparse_threshold {
        // Sparse reading for large files - sample every 64KB to minimize I/O while still warming EBS
        debug!("Using sparse direct I/O for large file ({} bytes)", file_size);
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
                // Align offset to page boundary for O_DIRECT requirement
                let aligned_offset = (offset / ALIGNMENT as u64) * ALIGNMENT as u64;
                
                if let Err(e) = file.seek(std::io::SeekFrom::Start(aligned_offset)).await {
                    debug!("Failed to seek to offset {}: {}", aligned_offset, e);
                    break;
                }
                
                let buffer_slice = unsafe { std::slice::from_raw_parts_mut(buffer, ALIGNMENT) };
                match file.read(buffer_slice).await {
                    Ok(n) => {
                        if n == 0 { break; }
                        samples_read += 1;
                    }
                    Err(e) => {
                        debug!("Failed to read at offset {}: {}", aligned_offset, e);
                        break;
                    }
                }
                offset += sample_interval;
            }
            Ok(())
        }.await;
        
        unsafe { std::alloc::dealloc(buffer, layout) };
        debug!("Sparse direct I/O completed: {} samples in {:?}", samples_read, _start.elapsed());
        
        match result {
            Ok(()) => Ok(WarmingResult {
                method: "tokio_direct_sparse",
                success: true,
                duration: _start.elapsed(),
            }),
            Err(e) => Err(e),
        }
    } else {
        // Full direct I/O reading for smaller files
        debug!("Using full direct I/O for file ({} bytes)", file_size);
        
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
                
                // Align read size to sector boundary for O_DIRECT
                let aligned_read_size = ((read_size + ALIGNMENT as u64 - 1) / ALIGNMENT as u64) * ALIGNMENT as u64;
                let actual_read_size = std::cmp::min(aligned_read_size, CHUNK_SIZE as u64) as usize;
                
                if let Err(e) = file.seek(std::io::SeekFrom::Start(offset)).await {
                    debug!("Failed to seek to offset {}: {}", offset, e);
                    break;
                }
                
                let buffer_slice = unsafe { std::slice::from_raw_parts_mut(buffer, actual_read_size) };
                match file.read(buffer_slice).await {
                    Ok(0) => break,
                    Ok(n) => {
                        total_read += n as u64;
                        offset += n as u64;
                    }
                    Err(e) => {
                        debug!("Failed to read chunk at offset {}: {}", offset, e);
                        break;
                    }
                }
            }
            Ok(total_read)
        }.await;
        
        unsafe { std::alloc::dealloc(buffer, layout) };
        
        match result {
            Ok(bytes_read) => {
                debug!("Full direct I/O completed: {} bytes read in {:?}", bytes_read, _start.elapsed());
                Ok(WarmingResult {
                    method: "tokio_direct_full",
                    success: true,
                    duration: _start.elapsed(),
                })
            }
            Err(e) => Err(e),
        }
    }
}

async fn warm_with_manual_reading(
    path: &PathBuf,
    file_size: u64,
    sparse_threshold: u64,
) -> Result<WarmingResult, std::io::Error> {
    let _start = Instant::now();
    let mut file = File::open(path).await?;
    
    let method = if sparse_threshold > 0 && file_size > sparse_threshold {
        debug!("Using sparse reading for large file: {} ({} bytes)", path.display(), file_size);
        let page_size: u64 = 4096;
        let mut offset: u64 = 0;
        let mut pages_read = 0;

        while offset < file_size {
            if let Err(e) = file.seek(std::io::SeekFrom::Start(offset)).await {
                debug!("Failed to seek in file {} at offset {}: {}", path.display(), offset, e);
                break;
            }
            let mut byte = [0; 1];
            match file.read(&mut byte).await {
                Ok(n) => {
                    if n == 0 {
                        break;
                    }
                    pages_read += 1;
                }
                Err(e) => {
                    debug!("Failed to read byte in file {} at offset {}: {}", path.display(), offset, e);
                    break;
                }
            }
            offset += page_size;
        }
        debug!("Sparse read completed: {} pages sampled in {:?}", pages_read, _start.elapsed());
        
                 // Drop pages from cache after sparse reading (we only wanted EBS warming)
         #[cfg(target_os = "linux")]
         {
             use std::os::unix::prelude::AsRawFd;
             let fd = file.as_raw_fd();
            let drop_result = posix_fadvise(fd, 0, file_size as i64, PosixFadviseAdvice::POSIX_FADV_DONTNEED);
            debug!("Sparse read cache drop result: {:?}", drop_result.is_ok());
        }
        
        "tokio_sparse"
    } else {
        debug!("Using full buffer read for file: {} ({} bytes)", path.display(), file_size);
        let mut reader = BufReader::new(file);
        let mut buffer = [0; 8192];
        let mut total_read = 0;

        loop {
            match reader.read(&mut buffer).await {
                Ok(0) => break,
                Ok(n) => { total_read += n; },
                Err(e) => {
                    debug!("Failed to read file {}: {}", path.display(), e);
                    break;
                }
            }
        }
        debug!("Full read completed: {} bytes in {:?}", total_read, _start.elapsed());
        
                 // Drop pages from cache after full reading (we only wanted EBS warming)
         #[cfg(target_os = "linux")]
         {
             use std::os::unix::prelude::AsRawFd;
             let inner_file = reader.into_inner();
             let fd = inner_file.as_raw_fd();
            let drop_result = posix_fadvise(fd, 0, file_size as i64, PosixFadviseAdvice::POSIX_FADV_DONTNEED);
            debug!("Full read cache drop result: {:?}", drop_result.is_ok());
        }
        
        "tokio_full"
    };
    
    Ok(WarmingResult {
        method,
        success: true,
        duration: _start.elapsed(),
    })
} 