use std::path::PathBuf;
use log::debug;

pub mod fallback;
pub mod tokio_async;

#[cfg(all(target_os = "linux", feature = "libaio"))]
pub mod libaio;

#[cfg(all(target_os = "linux", feature = "io_uring"))]
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
    
    #[cfg(all(target_os = "linux", feature = "io_uring"))]
    if options.use_io_uring {
        debug!("Using io_uring strategy for {}", path.display());
        return io_uring::warm_file(path, file_size, options).await;
    }
    
    #[cfg(all(target_os = "linux", feature = "libaio"))]
    if options.use_libaio {
        debug!("Using libaio strategy for {}", path.display());
        return libaio::warm_file(path, file_size, options).await;
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