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
use log::{debug, info};
use std::time::Instant;
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
    
    // Spawn file discovery task
    let discovery_args = Arc::clone(&args);
    let discovery_handle = tokio::spawn(async move {
        let mut file_count = 0u64;
        
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
                            if tx.send(entry.into_path()).is_err() {
                                debug!("Receiver dropped, stopping file discovery");
                                return file_count;
                            }
                            file_count += 1;
                        }
                    }
                    Err(err) => {
                        debug!("Failed to process directory entry: {}", err);
                    }
                }
            }
        }
        
        debug!("File discovery complete. {} files found.", file_count);
        file_count
    });

    let total_bytes_warmed = Arc::new(AtomicU64::new(0));
    let processed_files = Arc::new(AtomicU64::new(0));

    debug!("Starting concurrent file warming");
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

            async move {
                discovery_bar.inc(1);

                let mut file = match File::open(&path).await {
                    Ok(f) => f,
                    Err(e) => {
                        debug!("Failed to open file {}: {}", path.display(), e);
                        processed_files.fetch_add(1, Ordering::SeqCst);
                        warming_bar.inc(1);
                        return;
                    }
                };

                let file_size = match file.metadata().await {
                    Ok(metadata) => metadata.len(),
                    Err(e) => {
                        debug!("Failed to get metadata for {}: {}", path.display(), e);
                        processed_files.fetch_add(1, Ordering::SeqCst);
                        warming_bar.inc(1);
                        return;
                    }
                };

                if args.max_file_size > 0 && file_size > args.max_file_size {
                    debug!("Skipping large file: {} (size: {} > max: {})", path.display(), file_size, args.max_file_size);
                    processed_files.fetch_add(1, Ordering::SeqCst);
                    warming_bar.inc(1);
                    return;
                }

                let warmed = if cfg!(target_os = "linux") {
                    #[cfg(target_os = "linux")]
                    { warm_with_fadvise(&file, file_size) }
                    #[cfg(not(target_os = "linux"))]
                    { false }
                } else if cfg!(target_os = "macos") {
                    #[cfg(target_os = "macos")]
                    { warm_with_madvise(&file, file_size) }
                    #[cfg(not(target_os = "macos"))]
                    { false }
                } else {
                    false
                };

                if !warmed {
                    if args.sparse_large_files > 0 && file_size > args.sparse_large_files {
                        let page_size: u64 = 4096;
                        let mut offset: u64 = 0;

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
                                }
                                Err(e) => {
                                    debug!("Failed to read byte in file {} at offset {}: {}", path.display(), offset, e);
                                    break;
                                }
                            }
                            offset += page_size;
                        }
                    } else {
                        let mut reader = BufReader::new(file);
                        let mut buffer = [0; 8192];

                        loop {
                            match reader.read(&mut buffer).await {
                                Ok(0) => break,
                                Ok(_n) => {},
                                Err(e) => {
                                    debug!("Failed to read file {}: {}", path.display(), e);
                                    break;
                                }
                            }
                        }
                    }
                }

                total_bytes_warmed.fetch_add(file_size, Ordering::SeqCst);
                processed_files.fetch_add(1, Ordering::SeqCst);
                warming_bar.inc(1);
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
    
    info!(
        "Cache warming complete. Warmed {} bytes across {} files in {:.2?}.",
        total_bytes_warmed.load(Ordering::SeqCst),
        processed_files.load(Ordering::SeqCst),
        warming_duration
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
