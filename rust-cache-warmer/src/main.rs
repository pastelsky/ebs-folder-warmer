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
    
    if args.debug {
        println!("Configuration: {:?}", args);
    }

    let multi_progress = MultiProgress::new();
    let discovery_style = ProgressStyle::with_template(
        "{spinner:.green} [{elapsed_precise}] Discovering files: {pos}",
    )
    .unwrap();

    let discovery_bar = multi_progress.add(ProgressBar::new_spinner());
    discovery_bar.set_style(discovery_style);
    discovery_bar.enable_steady_tick(std::time::Duration::from_millis(100));

    let args = Arc::new(args);
    let mut file_paths = Vec::new();

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
                        file_paths.push(entry.into_path());
                        discovery_bar.inc(1);
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
    discovery_bar.finish_with_message(format!("Discovered {} files", file_paths.len()));

    let warming_style = ProgressStyle::with_template(
        "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] Warming files: {pos}/{len} ({percent}%)",
    )
    .unwrap()
    .progress_chars("#>-");

    let warming_bar = multi_progress.add(ProgressBar::new(file_paths.len() as u64));
    warming_bar.set_style(warming_style);

    let semaphore = Arc::new(Semaphore::new(args.queue_depth));
    let total_bytes_warmed = Arc::new(AtomicU64::new(0));

    let mut tasks = Vec::new();
    for path in file_paths {
        let semaphore = semaphore.clone();
        let warming_bar = warming_bar.clone();
        let total_bytes_warmed = total_bytes_warmed.clone();
        let args_clone = Arc::clone(&args);

        let task = tokio::spawn(async move {
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

            let mut reader = BufReader::new(file);
            let mut buffer = [0; 8192];

            loop {
                match reader.read(&mut buffer).await {
                    Ok(0) => break,
                    Ok(_) => (),
                    Err(e) => {
                        if args_clone.debug {
                            eprintln!("Failed to read file {}: {}", path.display(), e);
                        }
                        break;
                    }
                }
            }

            total_bytes_warmed.fetch_add(file_size, Ordering::SeqCst);
            warming_bar.inc(1);
        });
        tasks.push(task);
    }
    
    stream::iter(tasks)
        .for_each_concurrent(None, |task| async {
            let _ = task.await;
        })
        .await;


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
