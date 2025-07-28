use anyhow::Result;
use clap::Parser;
use futures::stream::{self, StreamExt};
use ignore::WalkBuilder;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use log::{debug, info, warn};
use std::time::{Instant, Duration};
use tokio::sync::{Semaphore, mpsc};

mod warming;
use warming::{WarmingOptions, warm_file};

#[derive(Parser, Debug)]
#[clap(
    name = "rust-cache-warmer",
    version = "1.2.0",
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

    #[clap(long, default_value = "1000", help = "Number of files to process per async task batch. Higher values reduce coordination overhead for small files.")]
    batch_size: usize,

    #[clap(long, help = "Use direct I/O (O_DIRECT) to bypass OS page cache. Ideal for EBS warming from S3 where you don't want data cached in memory.")]
    direct_io: bool,

    #[clap(long, help = "Use io_uring for high-performance async I/O (requires Linux 5.1+ and container support). Can achieve much higher queue depths than regular async I/O.")]
    io_uring: bool,

    #[clap(long, help = "Use Linux AIO (libaio) for high-performance async I/O. More widely supported than io_uring but slightly lower performance.")]
    libaio: bool,
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
    
    // Convert CLI options to WarmingOptions
    let warming_options = WarmingOptions {
        use_io_uring: args.io_uring,
        use_libaio: args.libaio,
        use_direct_io: args.direct_io,
        sparse_large_files: args.sparse_large_files,
    };
    
    // Display strategy selection at startup
    if warming_options.use_io_uring || warming_options.use_libaio {
        println!("🔧 Cache Warming Strategy:");
        if warming_options.use_io_uring {
            #[cfg(target_os = "linux")]
            println!("   📡 io_uring requested - will attempt for maximum performance");
            #[cfg(not(target_os = "linux"))]
            println!("   ⚠️  io_uring requested but not available on this platform");
        }
        if warming_options.use_libaio {
            #[cfg(target_os = "linux")]
            println!("   🚀 libaio requested - will attempt for high performance");
            #[cfg(not(target_os = "linux"))]
            println!("   ⚠️  libaio requested but not available on this platform");
        }
        if warming_options.use_direct_io {
            println!("   💾 Direct I/O enabled - bypassing OS page cache");
        }
        println!("   🔄 Will fall back to OS hints and Tokio async I/O if needed");
        println!();
    } else {
        println!("🔧 Cache Warming Strategy: Using OS hints and Tokio async I/O");
        if warming_options.use_direct_io {
            println!("   💾 Direct I/O enabled");
        }
        println!();
    }
    
    // Use a channel-based approach for batch file processing
    let (tx, rx) = mpsc::unbounded_channel::<Vec<PathBuf>>();
    
    // Spawn file discovery task
    let discovery_args = Arc::clone(&args);
    let discovery_handle = tokio::spawn(async move {
        let mut file_count = 0u64;
        let mut current_batch = Vec::with_capacity(discovery_args.batch_size);
        
        for path in &discovery_args.directories {
            debug!("Walking directory: {}", path.display());
            let mut walker_builder = WalkBuilder::new(path);
            let walker = walker_builder
                .threads(discovery_args.threads.unwrap_or_else(num_cpus::get))
                .follow_links(discovery_args.follow_symlinks)
                .max_depth(discovery_args.max_depth)
                .git_ignore(!discovery_args.respect_gitignore)
                .hidden(discovery_args.ignore_hidden)
                .build();

            for result in walker {
                match result {
                    Ok(entry) => {
                        if entry.file_type().map_or(false, |ft| ft.is_file()) {
                            current_batch.push(entry.into_path());
                            file_count += 1;
                            
                            // Send batch when it reaches the configured size
                            if current_batch.len() >= discovery_args.batch_size {
                                if tx.send(current_batch.clone()).is_err() {
                                    debug!("Receiver dropped, stopping file discovery");
                                    return file_count;
                                }
                                current_batch.clear();
                            }
                        }
                    }
                    Err(err) => {
                        debug!("Failed to process directory entry: {}", err);
                    }
                }
            }
        }
        
        // Send any remaining files in the final batch
        if !current_batch.is_empty() {
            if tx.send(current_batch).is_err() {
                debug!("Receiver dropped during final batch send");
            }
        }
        
        debug!("File discovery complete. {} files found.", file_count);
        file_count
    });

    let semaphore = Arc::new(Semaphore::new(args.queue_depth));
    let total_bytes_warmed = Arc::new(AtomicU64::new(0));
    let processed_files = Arc::new(AtomicU64::new(0));

    debug!("Starting concurrent file warming");
    let warming_start = Instant::now();

    // Process file batches as they're discovered using a stream with controlled concurrency
    let batch_stream = stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|batch| (batch, rx))
    });

    batch_stream
        .for_each_concurrent(args.queue_depth, |file_batch| {
            let semaphore = semaphore.clone();
            let warming_bar = warming_bar.clone();
            let discovery_bar = discovery_bar.clone();
            let total_bytes_warmed = total_bytes_warmed.clone();
            let processed_files = processed_files.clone();
            let args_clone = Arc::clone(&args);
            let warming_options = warming_options.clone();

            async move {
                let batch_start = Instant::now();
                let batch_size = file_batch.len();
                
                // Acquire semaphore once per batch
                let acquire_start = Instant::now();
                let _permit = semaphore.acquire().await.unwrap();
                let wait_time = acquire_start.elapsed();
                if wait_time > Duration::from_millis(10) {
                    debug!("High semaphore wait time: {:?} for batch of {} files", wait_time, batch_size);
                }
                
                // Process each file in the batch
                for path in file_batch {
                    let task_start = Instant::now();
                    discovery_bar.inc(1);

                    // Get file metadata
                    let file_size = match tokio::fs::metadata(&path).await {
                        Ok(metadata) => metadata.len(),
                        Err(e) => {
                            debug!("Failed to get metadata for {}: {}", path.display(), e);
                            processed_files.fetch_add(1, Ordering::SeqCst);
                            warming_bar.inc(1);
                            continue;
                        }
                    };

                    // Log file size category for distribution analysis
                    let size_category = match file_size {
                        0..=4096 => "tiny",
                        4097..=65536 => "small", 
                        65537..=1048576 => "medium",
                        1048577..=104857600 => "large",
                        _ => "huge"
                    };
                    debug!("Processing {} file: {} ({} bytes)", size_category, path.display(), file_size);

                    if args_clone.max_file_size > 0 && file_size > args_clone.max_file_size {
                        debug!("Skipping large file: {} (size: {} > max: {})", path.display(), file_size, args_clone.max_file_size);
                        processed_files.fetch_add(1, Ordering::SeqCst);
                        warming_bar.inc(1);
                        continue;
                    }

                    // Use the modular warming interface
                    let _warming_start = Instant::now();
                    match warm_file(&path, file_size, &warming_options).await {
                        Ok(result) => {
                            debug!("File {} warming completed: method={}, success={}, duration={:?}, size={}", 
                                   path.display(), result.method, result.success, result.duration, file_size);
                            
                            // Log performance warnings for slow operations
                            if result.duration > Duration::from_millis(100) {
                                warn!("Slow warming operation: {} took {:?} for {} bytes", 
                                      path.display(), result.duration, file_size);
                            }
                        }
                        Err(e) => {
                            debug!("Failed to warm file {}: {}", path.display(), e);
                        }
                    }

                    total_bytes_warmed.fetch_add(file_size, Ordering::SeqCst);
                    processed_files.fetch_add(1, Ordering::SeqCst);
                    warming_bar.inc(1);
                    
                    let total_task_time = task_start.elapsed();
                    debug!("Total task time for {}: {:?}", path.display(), total_task_time);
                }
                
                let batch_duration = batch_start.elapsed();
                debug!("Completed batch of {} files in {:?}", batch_size, batch_duration);
            }
        })
        .await;

    // Wait for discovery to complete and get final count
    let total_files_discovered = discovery_handle.await.unwrap();
    
    debug!("File warming phase complete");
    let warming_duration = warming_start.elapsed();
    
    // Enhanced performance statistics
    let total_bytes = total_bytes_warmed.load(Ordering::SeqCst);
    let total_files = processed_files.load(Ordering::SeqCst);
    let throughput_mbps = if warming_duration.as_secs_f64() > 0.0 {
        (total_bytes as f64) / (1024.0 * 1024.0) / warming_duration.as_secs_f64()
    } else {
        0.0
    };
    let files_per_sec = if warming_duration.as_secs_f64() > 0.0 {
        total_files as f64 / warming_duration.as_secs_f64()
    } else {
        0.0
    };
    let avg_file_size = if total_files > 0 { total_bytes / total_files } else { 0 };
    
    debug!("Performance metrics:");
    debug!("  Total files discovered: {}", total_files_discovered);
    debug!("  Total files processed: {}", total_files);
    debug!("  Total bytes warmed: {} ({:.2} MB)", total_bytes, total_bytes as f64 / (1024.0 * 1024.0));
    debug!("  Warming duration: {:?}", warming_duration);
    debug!("  Throughput: {:.2} MB/s", throughput_mbps);
    debug!("  Files per second: {:.2}", files_per_sec);
    debug!("  Average file size: {} bytes", avg_file_size);
    debug!("  Queue depth: {}", args.queue_depth);
    debug!("  Concurrency efficiency: {:.1}%", (total_files as f64 / warming_duration.as_secs_f64() / args.queue_depth as f64) * 100.0);
    
    discovery_bar.finish_with_message(format!("Discovered {} files", total_files_discovered));
    warming_bar.finish_with_message(format!("Warmed {} files", processed_files.load(Ordering::SeqCst)));
    multi_progress.clear().unwrap();
    
    info!(
        "Cache warming complete. Warmed {} bytes ({:.2} MB) across {} files in {:.2?} at {:.2} MB/s.",
        total_bytes,
        total_bytes as f64 / (1024.0 * 1024.0),
        total_files,
        warming_duration,
        throughput_mbps
    );
    
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
