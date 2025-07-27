use anyhow::Result;
use clap::Parser;
use futures::stream::{self, StreamExt};
use ignore::WalkBuilder;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, BufReader, AsyncSeekExt};
use log::{debug, info, warn};
use std::time::{Instant, Duration};
use tokio::sync::mpsc;
#[cfg(target_os = "linux")]
use nix::fcntl::{posix_fadvise, PosixFadviseAdvice};
#[cfg(target_os = "macos")]
use nix::sys::mman::{madvise, MmapAdvise};
use std::os::unix::prelude::AsRawFd;
use std::ptr::NonNull;

#[derive(Parser, Debug)]
#[clap(
    name = "rust-cache-warmer",
    version = "1.0.0",
    author = "AI Assistant",
    about = "A high-performance, concurrent file cache warmer written in Rust."
)]
struct Opts {
    #[clap(
        short,
        long,
        default_value_t = 32,
        help = "Number of concurrent files to read at once. Lower values reduce disk queue pressure."
    )]
    queue_depth: usize,

    #[clap(
        short = 'T',
        long,
        help = "Number of threads for file discovery. Defaults to number of logical cores."
    )]
    threads: Option<usize>,

    #[clap(
        required = true,
        help = "One or more directory paths to warm.",
        num_args = 1..
    )]
    directories: Vec<PathBuf>,

    #[clap(long, help = "Follow symbolic links.")]
    follow_symlinks: bool,

    #[clap(
        long,
        help = "Respect .gitignore, .ignore, and other ignore files. Disabled by default."
    )]
    respect_gitignore: bool,

    #[clap(
        long,
        value_name = "DEPTH",
        help = "Maximum directory traversal depth."
    )]
    max_depth: Option<usize>,

    #[clap(long, help = "Print detailed debug information.")]
    debug: bool,
    
    #[clap(long, help = "Enable profiling and generate a flamegraph.svg")]
    profile: bool,

    #[clap(long, help = "Ignore hidden files and directories (those starting with '.'). Disabled by default.")]
    ignore_hidden: bool,

    #[clap(long, default_value = "0", help = "Skip files larger than this size in bytes (0 means no limit).")]
    max_file_size: u64,

    #[clap(long, default_value = "0", help = "Use sparse reading for files larger than this size in bytes (0 means disabled). Reads 1 byte every 4096 bytes to warm cache efficiently.")]
    sparse_large_files: u64,
}

#[cfg(target_os = "linux")]
fn warm_with_fadvise(file: &File, file_size: u64) -> bool {
    let fd = file.as_raw_fd();
    posix_fadvise(fd, 0, file_size as i64, PosixFadviseAdvice::POSIX_FADV_WILLNEED).is_ok()
}
#[cfg(target_os = "macos")]
fn warm_with_madvise(file: &File, file_size: u64) -> bool {
    let fd = file.as_raw_fd();
    let ptr = unsafe { nix::libc::mmap(std::ptr::null_mut(), file_size as usize, nix::libc::PROT_NONE, nix::libc::MAP_SHARED, fd, 0) };
    if ptr != nix::libc::MAP_FAILED {
        let nn_ptr = NonNull::new(ptr).expect("mmap returned non-null but failed to create NonNull");
        let res = unsafe { madvise(nn_ptr, file_size as usize, MmapAdvise::MADV_WILLNEED) };
        unsafe { nix::libc::munmap(ptr, file_size as usize) };
        res.is_ok()
    } else {
        false
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Opts::parse();

    // Start the profiler if the --profile flag is passed
    let guard = if args.profile {
        Some(pprof::ProfilerGuardBuilder::default()
            .frequency(1000) // Sample 1000 times per second
            .blocklist(&["libc", "libgcc", "pthread", "vdso"])
            .build()
            .unwrap())
    } else {
        None
    };
    
    // Initialize logger
    if args.debug {
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("debug")).init();
    } else {
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    }

    let total_start = Instant::now();
    debug!("Configuration: {:?}", args);
    debug!("Performance monitoring enabled. Queue depth: {}, Threads: {:?}", 
           args.queue_depth, args.threads.unwrap_or_else(num_cpus::get));

    let multi_progress = MultiProgress::new();
    let discovery_style = ProgressStyle::with_template(
        "{spinner:.green} [{elapsed_precise}] Processing files: {pos}",
    )
    .unwrap();

    let discovery_bar = multi_progress.add(ProgressBar::new_spinner());
    discovery_bar.set_style(discovery_style);
    discovery_bar.enable_steady_tick(std::time::Duration::from_millis(100));

    let warming_style = ProgressStyle::with_template(
        "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] Warmed files: {pos} ({rate}/s)",
    )
    .unwrap()
    .progress_chars("#>-");

    let warming_bar = multi_progress.add(ProgressBar::new_spinner());
    warming_bar.set_style(warming_style);

    let args = Arc::new(args);
    
    // Use a channel-based approach for streaming file processing
    let (tx, rx) = mpsc::unbounded_channel::<PathBuf>();
    
    // Performance monitoring counters
    let discovery_time = Arc::new(AtomicU64::new(0));
    let files_discovered = Arc::new(AtomicU64::new(0));
    let fadvise_success_count = Arc::new(AtomicU64::new(0));
    let fadvise_fail_count = Arc::new(AtomicU64::new(0));
    let sparse_read_count = Arc::new(AtomicU64::new(0));
    let full_read_count = Arc::new(AtomicU64::new(0));
    let file_errors = Arc::new(AtomicU64::new(0));
    let bytes_by_size_bucket = Arc::new([
        AtomicU64::new(0), // 0-1MB
        AtomicU64::new(0), // 1-10MB  
        AtomicU64::new(0), // 10-100MB
        AtomicU64::new(0), // 100MB-1GB
        AtomicU64::new(0), // >1GB
    ]);
    
    // Spawn file discovery task
    let discovery_args = Arc::clone(&args);
    let discovery_time_clone = discovery_time.clone();
    let files_discovered_clone = files_discovered.clone();
    let discovery_handle = tokio::spawn(async move {
        let discovery_start = Instant::now();
        let mut file_count = 0u64;
        let mut last_report = Instant::now();
        
        debug!("Starting file discovery across {} directories", discovery_args.directories.len());
        
        for (dir_idx, path) in discovery_args.directories.iter().enumerate() {
            debug!("Walking directory {} of {}: {}", dir_idx + 1, discovery_args.directories.len(), path.display());
            let dir_start = Instant::now();
            let mut walker_builder = WalkBuilder::new(path);
            let walker = walker_builder
                .threads(discovery_args.threads.unwrap_or_else(num_cpus::get))
                .follow_links(discovery_args.follow_symlinks)
                .max_depth(discovery_args.max_depth)
                .git_ignore(!discovery_args.respect_gitignore)
                .hidden(discovery_args.ignore_hidden)
                .build();

            let mut dir_file_count = 0u64;
            for result in walker {
                match result {
                    Ok(entry) => {
                        if entry.file_type().map_or(false, |ft| ft.is_file()) {
                            if tx.send(entry.into_path()).is_err() {
                                debug!("Receiver dropped, stopping file discovery");
                                break;
                            }
                            file_count += 1;
                            dir_file_count += 1;
                            
                            // Report discovery progress every 10,000 files
                            if file_count % 10_000 == 0 && last_report.elapsed() >= Duration::from_secs(5) {
                                debug!("Discovery progress: {} files found so far, rate: {:.0} files/sec", 
                                       file_count, file_count as f64 / discovery_start.elapsed().as_secs_f64());
                                last_report = Instant::now();
                            }
                        }
                    }
                    Err(err) => {
                        debug!("Failed to process directory entry: {}", err);
                    }
                }
            }
            
            debug!("Directory {} completed: {} files found in {:.2?}", 
                   path.display(), dir_file_count, dir_start.elapsed());
        }
        
        let discovery_elapsed = discovery_start.elapsed();
        discovery_time_clone.store(discovery_elapsed.as_millis() as u64, Ordering::SeqCst);
        files_discovered_clone.store(file_count, Ordering::SeqCst);
        
        debug!("File discovery complete: {} files in {:.2?} ({:.0} files/sec)", 
               file_count, discovery_elapsed, file_count as f64 / discovery_elapsed.as_secs_f64());
        file_count
    });

    let total_bytes_warmed = Arc::new(AtomicU64::new(0));
    let processed_files = Arc::new(AtomicU64::new(0));

    debug!("Starting concurrent file warming with {} workers", args.queue_depth);
    let warming_start = Instant::now();

    // Process files as they're discovered using a stream with controlled concurrency
    let file_stream = stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|path| (path, rx))
    });

    file_stream
        .for_each_concurrent(args.queue_depth, |path| {
            // Clone references outside the async block for better performance
            let warming_bar = warming_bar.clone();
            let discovery_bar = discovery_bar.clone();
            let total_bytes_warmed = total_bytes_warmed.clone();
            let processed_files = processed_files.clone();
            let args = args.clone();
            
            // Performance counters
            let fadvise_success = fadvise_success_count.clone();
            let fadvise_fail = fadvise_fail_count.clone();
            let sparse_reads = sparse_read_count.clone();
            let full_reads = full_read_count.clone();
            let errors = file_errors.clone();
            let size_buckets = bytes_by_size_bucket.clone();

            async move {
                let file_start = Instant::now();
                discovery_bar.inc(1);

                let mut file = match File::open(&path).await {
                    Ok(f) => f,
                    Err(e) => {
                        debug!("Failed to open file {}: {}", path.display(), e);
                        errors.fetch_add(1, Ordering::SeqCst);
                        processed_files.fetch_add(1, Ordering::SeqCst);
                        warming_bar.inc(1);
                        return;
                    }
                };

                let metadata_start = Instant::now();
                let file_size = match file.metadata().await {
                    Ok(metadata) => metadata.len(),
                    Err(e) => {
                        debug!("Failed to get metadata for {}: {} (took {:.2?})", 
                               path.display(), e, metadata_start.elapsed());
                        errors.fetch_add(1, Ordering::SeqCst);
                        processed_files.fetch_add(1, Ordering::SeqCst);
                        warming_bar.inc(1);
                        return;
                    }
                };
                
                // Track file size distribution
                let bucket_idx = match file_size {
                    0..=1_048_576 => 0,           // 0-1MB
                    1_048_577..=10_485_760 => 1,  // 1-10MB
                    10_485_761..=104_857_600 => 2, // 10-100MB
                    104_857_601..=1_073_741_824 => 3, // 100MB-1GB
                    _ => 4,                       // >1GB
                };
                size_buckets[bucket_idx].fetch_add(file_size, Ordering::SeqCst);

                if args.max_file_size > 0 && file_size > args.max_file_size {
                    debug!("Skipping large file: {} (size: {} > max: {})", path.display(), file_size, args.max_file_size);
                    processed_files.fetch_add(1, Ordering::SeqCst);
                    warming_bar.inc(1);
                    return;
                }

                let warming_method_start = Instant::now();
                let warmed = if cfg!(target_os = "linux") {
                    #[cfg(target_os = "linux")]
                    { 
                        let success = warm_with_fadvise(&file, file_size);
                        if success {
                            fadvise_success.fetch_add(1, Ordering::SeqCst);
                            debug!("fadvise SUCCESS for {} ({} bytes) in {:.2?}", 
                                   path.display(), file_size, warming_method_start.elapsed());
                        } else {
                            fadvise_fail.fetch_add(1, Ordering::SeqCst);
                            debug!("fadvise FAILED for {} ({} bytes) in {:.2?}", 
                                   path.display(), file_size, warming_method_start.elapsed());
                        }
                        success
                    }
                    #[cfg(not(target_os = "linux"))]
                    { false }
                } else if cfg!(target_os = "macos") {
                    #[cfg(target_os = "macos")]
                    { 
                        let success = warm_with_madvise(&file, file_size);
                        if success {
                            fadvise_success.fetch_add(1, Ordering::SeqCst);
                            debug!("madvise SUCCESS for {} ({} bytes) in {:.2?}", 
                                   path.display(), file_size, warming_method_start.elapsed());
                        } else {
                            fadvise_fail.fetch_add(1, Ordering::SeqCst);
                            debug!("madvise FAILED for {} ({} bytes) in {:.2?}", 
                                   path.display(), file_size, warming_method_start.elapsed());
                        }
                        success
                    }
                    #[cfg(not(target_os = "macos"))]
                    { false }
                } else {
                    false
                };

                if !warmed {
                    let read_start = Instant::now();
                    if args.sparse_large_files > 0 && file_size > args.sparse_large_files {
                        sparse_reads.fetch_add(1, Ordering::SeqCst);
                        debug!("Using SPARSE read for {} ({} bytes)", path.display(), file_size);
                        
                        let page_size: u64 = 4096;
                        let mut offset: u64 = 0;
                        let mut pages_read = 0u64;

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
                        
                        debug!("Sparse read completed for {} in {:.2?}: {} pages read", 
                               path.display(), read_start.elapsed(), pages_read);
                    } else {
                        full_reads.fetch_add(1, Ordering::SeqCst);
                        debug!("Using FULL read for {} ({} bytes)", path.display(), file_size);
                        
                        let mut reader = BufReader::new(file);
                        let mut buffer = [0; 8192];
                        let mut bytes_read = 0u64;

                        loop {
                            match reader.read(&mut buffer).await {
                                Ok(0) => break,
                                Ok(n) => {
                                    bytes_read += n as u64;
                                },
                                Err(e) => {
                                    debug!("Failed to read file {}: {}", path.display(), e);
                                    break;
                                }
                            }
                        }
                        
                        debug!("Full read completed for {} in {:.2?}: {} bytes read", 
                               path.display(), read_start.elapsed(), bytes_read);
                    }
                }

                total_bytes_warmed.fetch_add(file_size, Ordering::SeqCst);
                processed_files.fetch_add(1, Ordering::SeqCst);
                warming_bar.inc(1);
                
                let file_duration = file_start.elapsed();
                if file_duration > Duration::from_millis(100) {
                    debug!("SLOW file processing: {} took {:.2?} ({} bytes, {:.2} MB/s)", 
                           path.display(), file_duration, file_size, 
                           (file_size as f64 / 1_048_576.0) / file_duration.as_secs_f64());
                }
            }
        })
        .await;

    // Wait for discovery to complete and get final count
    let total_files_discovered = discovery_handle.await.unwrap();
    
    debug!("File warming phase complete");
    let warming_duration = warming_start.elapsed();
    
    discovery_bar.finish_with_message(format!("Discovered {} files", total_files_discovered));
    warming_bar.finish_with_message(format!("Warmed {} files", processed_files.load(Ordering::SeqCst)));
    multi_progress.clear().unwrap();

    // Performance summary 
    let total_bytes = total_bytes_warmed.load(Ordering::SeqCst);
    let processed_count = processed_files.load(Ordering::SeqCst);
    let discovery_ms = discovery_time.load(Ordering::SeqCst);

    info!("=== PERFORMANCE SUMMARY ===");
    info!("Files: {} discovered, {} processed", total_files_discovered, processed_count);
    info!("Data: {} bytes ({:.2} GB) warmed", total_bytes, total_bytes as f64 / 1_073_741_824.0);
    info!("Timing: discovery {:.2?}, warming {:.2?}, total {:.2?}", 
          Duration::from_millis(discovery_ms), warming_duration, total_start.elapsed());
    info!("Throughput: {:.0} files/sec, {:.2} MB/sec", 
          processed_count as f64 / warming_duration.as_secs_f64(),
          (total_bytes as f64 / 1_048_576.0) / warming_duration.as_secs_f64());
    
    // Method effectiveness
    let fadvise_success = fadvise_success_count.load(Ordering::SeqCst);
    let fadvise_fails = fadvise_fail_count.load(Ordering::SeqCst);
    let sparse_reads = sparse_read_count.load(Ordering::SeqCst);
    let full_reads = full_read_count.load(Ordering::SeqCst);
    let errors = file_errors.load(Ordering::SeqCst);
    
    info!("Method breakdown: {} OS-native success, {} OS-native fails, {} sparse reads, {} full reads",
          fadvise_success, fadvise_fails, sparse_reads, full_reads);
    
    if errors > 0 {
        warn!("File errors: {} files failed to process", errors);
    }
    
    // File size distribution
    debug!("File size distribution:");
    debug!("  0-1MB: {:.2} MB", bytes_by_size_bucket[0].load(Ordering::SeqCst) as f64 / 1_048_576.0);
    debug!("  1-10MB: {:.2} MB", bytes_by_size_bucket[1].load(Ordering::SeqCst) as f64 / 1_048_576.0);
    debug!("  10-100MB: {:.2} MB", bytes_by_size_bucket[2].load(Ordering::SeqCst) as f64 / 1_048_576.0);
    debug!("  100MB-1GB: {:.2} MB", bytes_by_size_bucket[3].load(Ordering::SeqCst) as f64 / 1_048_576.0);
    debug!("  >1GB: {:.2} MB", bytes_by_size_bucket[4].load(Ordering::SeqCst) as f64 / 1_048_576.0);
    
    // If profiling was enabled, generate the report.
    if let Some(guard) = guard {
        if let Ok(report) = guard.report().build() {
            let file = std::fs::File::create("flamegraph.svg").unwrap();
            report.flamegraph(file).unwrap();
            info!("Profiling complete. Flamegraph saved to flamegraph.svg");
        };
    }

    debug!("All phases complete. Exiting.");
    let total_duration = total_start.elapsed();
    if !args.debug {
        println!("Total execution time: {:.2?}", total_duration);
    }

    Ok(())
}
