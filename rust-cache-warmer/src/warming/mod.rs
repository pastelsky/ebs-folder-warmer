use std::path::PathBuf;
use log::debug;

pub mod fallback;
pub mod tokio_async;

#[cfg(target_os = "linux")]
pub mod libaio;

#[cfg(target_os = "linux")]
pub mod io_uring;

/// Warming strategy options
#[derive(Debug, Clone)]
pub struct WarmingOptions {
    pub use_io_uring: bool,
    pub use_libaio: bool,
    pub use_direct_io: bool,
    pub sparse_large_files: u64,
}

/// Result of a warming operation
#[derive(Debug)]
pub struct WarmingResult {
    pub method: &'static str,
    pub success: bool,
    pub duration: std::time::Duration,
}

/// Main warming function that selects the best strategy
pub async fn warm_file(
    path: &PathBuf,
    file_size: u64,
    options: &WarmingOptions,
) -> Result<WarmingResult, std::io::Error> {
    let _start = std::time::Instant::now();
    
    // Strategy selection priority:
    // 1. io_uring (if available and requested)
    // 2. libaio (if available and requested)
    // 3. OS hints (fadvise/madvise)
    // 4. Tokio fallback
    
    #[cfg(target_os = "linux")]
    if options.use_io_uring {
        debug!("Attempting io_uring strategy for {}", path.display());
        match io_uring::warm_file(path, file_size, options).await {
            Ok(result) => {
                return Ok(result);
            }
            Err(e) if e.kind() == std::io::ErrorKind::Unsupported => {
                debug!("io_uring not available: {}", e);
            }
            Err(e) => return Err(e),
        }
    }
    
    #[cfg(target_os = "linux")]
    if options.use_libaio {
        debug!("Attempting libaio strategy for {}", path.display());
        match libaio::warm_file(path, file_size, options).await {
            Ok(result) => {
                return Ok(result);
            }
            Err(e) if e.kind() == std::io::ErrorKind::Unsupported => {
                debug!("libaio not available: {}", e);
            }
            Err(e) => return Err(e),
        }
    }
    
    // Try OS hints first (most efficient)
    debug!("Trying OS hints (fadvise/madvise) for {}", path.display());
    if let Ok(result) = fallback::warm_with_os_hints(path, file_size).await {
        if result.success {
            return Ok(result);
        }
    }
    
    // Fallback to Tokio async I/O
    debug!("Using Tokio async I/O for {}", path.display());
    tokio_async::warm_file(path, file_size, options).await
} 