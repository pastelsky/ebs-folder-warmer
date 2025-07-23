use anyhow::Result;
use clap::Parser;
use futures::stream::{self, StreamExt};
use ignore::WalkBuilder;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::fs::File;
use tokio::io::AsyncReadExt;
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

    let mut file_paths = Vec::new();
    for path in &opts.directories {
        let mut walker_builder = WalkBuilder::new(path);
        walker_builder
            .threads(opts.threads.unwrap_or_else(num_cpus::get))
            .follow_links(opts.follow_symlinks)
            .max_depth(opts.max_depth);

        // By default, do not respect ignore files.
        if !opts.respect_gitignore {
            walker_builder
                .ignore(false)
                .git_ignore(false)
                .git_global(false)
                .git_exclude(false)
                .parents(false);
        }

        let walker = walker_builder.build();
        for result in walker {
            match result {
                Ok(entry) => {
                    if entry.file_type().map_or(false, |ft| ft.is_file()) {
                        file_paths.push(entry.into_path());
                        discovery_bar.inc(1);
                    }
                }
                Err(err) => {
                    if opts.debug {
                        eprintln!("[DEBUG] Discovery error: {}", err);
                    }
                }
            }
        }
    }
    discovery_bar.finish_with_message(format!("Discovered {} files", file_paths.len()));

    let warming_bar =
        multi_progress.add(ProgressBar::new(file_paths.len() as u64));
    warming_bar.set_style(
        ProgressStyle::with_template(
            "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({percent}%)",
        )
        .unwrap()
        .progress_chars("#>-"),
    );

    let semaphore = Arc::new(Semaphore::new(opts.queue_depth));
    let mut tasks = Vec::new();

    for path in file_paths {
        let semaphore_clone = Arc::clone(&semaphore);
        let warming_bar_clone = warming_bar.clone();
        let debug_mode = opts.debug;
        let task = tokio::spawn(async move {
            let _permit = semaphore_clone.acquire().await.unwrap();
            let mut file = match File::open(&path).await {
                Ok(f) => f,
                Err(err) => {
                    if debug_mode {
                        eprintln!("[DEBUG] Failed to open file {:?}: {}", path, err);
                    }
                    return;
                }
            };
            let mut buffer = Vec::new();
            if let Err(err) = file.read_to_end(&mut buffer).await {
                if debug_mode {
                    eprintln!("[DEBUG] Failed to read file {:?}: {}", path, err);
                }
            }
            warming_bar_clone.inc(1);
        });
        tasks.push(task);
    }
    
    stream::iter(tasks)
        .for_each_concurrent(None, |task| async {
            let _ = task.await;
        })
        .await;

    warming_bar.finish_with_message("Warming complete!");
    multi_progress.clear().unwrap();

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
