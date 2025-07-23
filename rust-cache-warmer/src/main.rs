use anyhow::Result;
use clap::Parser;
use futures::stream::{self, StreamExt};
use ignore::WalkBuilder;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, BufReader};
use tokio::sync::Semaphore;

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
        default_value_t = 128,
        help = "Number of concurrent files to read at once."
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
}

#[tokio::main]
async fn main() -> Result<()> {
    let opts: Opts = Opts::parse();
    
    // Start the profiler if the --profile flag is passed
    let guard = if opts.profile {
        Some(pprof::ProfilerGuardBuilder::default()
            .frequency(1000) // Sample 1000 times per second
            .blocklist(&["libc", "libgcc", "pthread", "vdso"])
            .build()
            .unwrap())
    } else {
        None
    };
    
    if opts.debug {
        println!("Configuration: {:?}", opts);
    }

    let multi_progress = Arc::new(MultiProgress::new());

    let discovery_bar = multi_progress.add(ProgressBar::new_spinner());
    discovery_bar.set_style(
        ProgressStyle::with_template("{spinner:.green} Discovering files: {pos} found...")
            .unwrap(),
    );
    discovery_bar.enable_steady_tick(std::time::Duration::from_millis(100));

    let warming_bar =
        multi_progress.add(ProgressBar::new(0)); // Initialize with 0, will be updated after discovery

    let semaphore = Arc::new(Semaphore::new(opts.queue_depth));
    let total_bytes_warmed = Arc::new(AtomicU64::new(0));
    let args = Arc::new(opts);

    for path in &args.directories {
        let mut walker_builder = WalkBuilder::new(path);
        let walker = walker_builder
            .threads(args.threads.unwrap_or_else(num_cpus::get))
            .follow_links(args.follow_symlinks)
            .max_depth(args.max_depth)
            .git_ignore(!args.respect_gitignore)
            .build();

        for result in walker {
            match result {
                Ok(entry) => {
                    if entry.file_type().map_or(false, |ft| ft.is_file()) {
                        let path = entry.into_path();
                        let semaphore = semaphore.clone();
                        let warming_bar = warming_bar.clone();
                        let total_bytes_warmed = total_bytes_warmed.clone();
                        let args_clone = Arc::clone(&args);

                        tokio::spawn(async move {
                            let _permit = semaphore.acquire().await.unwrap();

                            let file = match File::open(&path).await {
                                Ok(f) => f,
                                Err(e) => {
                                    if args_clone.debug {
                                        eprintln!("Failed to open file {}: {}", path.display(), e);
                                    }
                                    return;
                                }
                            };

                            let file_size = match file.metadata().await {
                                Ok(metadata) => metadata.len(),
                                Err(_) => 0,
                            };
                            
                            // Use a buffered reader to read the file in chunks.
                            // This is more memory-efficient than read_to_end for large files
                            // and avoids allocating a large buffer for every file.
                            let mut reader = BufReader::new(file);
                            let mut buffer = [0; 8192]; // 8KB buffer

                            loop {
                                match reader.read(&mut buffer).await {
                                    Ok(0) => break, // EOF reached.
                                    Ok(_) => (),    // Bytes read, continue.
                                    Err(e) => {
                                        if args_clone.debug {
                                            eprintln!("Failed to read file {}: {}", path.display(), e);
                                        }
                                        break; // Stop reading this file on error.
                                    }
                                }
                            }

                            total_bytes_warmed.fetch_add(file_size, Ordering::SeqCst);
                            warming_bar.inc(1);
                        });
                    }
                }
                Err(err) => {
                    if args.debug {
                        eprintln!("[DEBUG] Failed to process directory entry: {}", err);
                    }
                }
            }
        }
    }

    discovery_bar.finish_with_message("Discovery complete");

    // Wait for all warming tasks to complete. This is a simple way to wait.
    // A more robust solution might use a channel or another synchronization primitive.
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    while Arc::strong_count(&semaphore) > 1 {
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }


    warming_bar.finish_with_message("Done");
    multi_progress.clear().unwrap();
    println!("Cache warming complete. Warmed {} bytes.", total_bytes_warmed.load(Ordering::SeqCst));
    
    // If profiling was enabled, generate the report.
    if let Some(guard) = guard {
        if let Ok(report) = guard.report().build() {
            let file = std::fs::File::create("flamegraph.svg").unwrap();
            report.flamegraph(file).unwrap();
            println!("Profiling complete. Flamegraph saved to flamegraph.svg");
        };
    }

    Ok(())
}
