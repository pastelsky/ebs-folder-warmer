use std::path::PathBuf;
use std::os::unix::prelude::AsRawFd;
use std::ptr::NonNull;
use std::time::Instant;
use tokio::fs::File;
use log::debug;

#[cfg(target_os = "linux")]
use nix::fcntl::{posix_fadvise, PosixFadviseAdvice};
#[cfg(target_os = "macos")]
use nix::sys::mman::{madvise, MmapAdvise};

use crate::warming::WarmingResult;

pub async fn warm_with_os_hints(
    path: &PathBuf,
    file_size: u64,
) -> Result<WarmingResult, std::io::Error> {
    let start = Instant::now();
    
    let file = File::open(path).await?;
    
    let (method, success) = if cfg!(target_os = "linux") {
        #[cfg(target_os = "linux")]
        {
            let result = warm_with_fadvise(&file, file_size);
            ("linux_fadvise", result)
        }
        #[cfg(not(target_os = "linux"))]
        { ("fadvise_unavailable", false) }
    } else if cfg!(target_os = "macos") {
        #[cfg(target_os = "macos")]
        {
            let result = warm_with_madvise(&file, file_size);
            ("macos_madvise", result)
        }
        #[cfg(not(target_os = "macos"))]
        { ("madvise_unavailable", false) }
    } else {
        ("os_hints_unsupported", false)
    };
    
    Ok(WarmingResult {
        method,
        success,
        duration: start.elapsed(),
    })
}

#[cfg(target_os = "linux")]
fn warm_with_fadvise(file: &File, file_size: u64) -> bool {
    let start = Instant::now();
    let fd = file.as_raw_fd();
    
    // Step 1: Tell OS to read data (triggers EBS fetch from S3)
    let warm_result = posix_fadvise(fd, 0, file_size as i64, PosixFadviseAdvice::POSIX_FADV_WILLNEED).is_ok();
    
    if warm_result {
        // Step 2: Immediately drop from cache (we only wanted EBS warming, not OS caching)
        let drop_result = posix_fadvise(fd, 0, file_size as i64, PosixFadviseAdvice::POSIX_FADV_DONTNEED).is_ok();
        debug!("fadvise WILLNEED+DONTNEED took {:?}, warm: {}, drop: {}", start.elapsed(), warm_result, drop_result);
        
        // Success if we managed to warm (drop is less critical)
        warm_result
    } else {
        debug!("fadvise WILLNEED failed in {:?}", start.elapsed());
        false
    }
}

#[cfg(target_os = "macos")]
fn warm_with_madvise(file: &File, file_size: u64) -> bool {
    let start = Instant::now();
    let fd = file.as_raw_fd();
    let ptr = unsafe { nix::libc::mmap(std::ptr::null_mut(), file_size as usize, nix::libc::PROT_NONE, nix::libc::MAP_SHARED, fd, 0) };
    if ptr != nix::libc::MAP_FAILED {
        let nn_ptr = NonNull::new(ptr).expect("mmap returned non-null but failed to create NonNull");
        
        // Step 1: Tell OS to read data (triggers EBS fetch from S3)
        let warm_result = unsafe { madvise(nn_ptr, file_size as usize, MmapAdvise::MADV_WILLNEED) };
        
        if warm_result.is_ok() {
            // Step 2: Immediately drop from cache (we only wanted EBS warming, not OS caching)
            let drop_result = unsafe { madvise(nn_ptr, file_size as usize, MmapAdvise::MADV_FREE) };
            debug!("madvise WILLNEED+FREE took {:?}, warm: {}, drop: {}", start.elapsed(), warm_result.is_ok(), drop_result.is_ok());
        }
        
        unsafe { nix::libc::munmap(ptr, file_size as usize) };
        warm_result.is_ok()
    } else {
        debug!("mmap failed for madvise operation");
        false
    }
} 