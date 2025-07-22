#include "page_cache_warmer.h"
#include <sys/resource.h>
#ifdef __linux__
#include <sys/syscall.h>
#endif
#include <sys/stat.h>
#include <fcntl.h>
#include <unistd.h>
#include <errno.h>
#include <stdarg.h>
#include <stddef.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <sys/time.h>
#include <stdio.h>
#include <getopt.h>
#include <syslog.h>
#include <sys/mman.h>
#include <sys/types.h>

// Global state (minimized)
time_t g_last_log_time = 0;

// Configuration functions
void config_init(struct config *cfg)
{
    cfg->read_size_kb = DEFAULT_READ_SIZE_KB;
    cfg->queue_depth = DEFAULT_QUEUE_DEPTH;
    cfg->silent_mode = 0;
    cfg->syslog_mode = 0;
    cfg->num_directories = 0;
    cfg->directories = NULL;
    cfg->debug_mode = 0;
    cfg->max_depth = -1;
    cfg->num_threads = 1; // Default single-threaded
    cfg->throttle = 0;
}

void config_print_help(void)
{
    printf("Usage: page-cache-warmer [OPTIONS] <directory1> [directory2 ...]\n");
    printf("High-performance page cache warming utility.\n");
    printf("Recursively reads all files in the specified directories to load them into the OS page cache.\n");
    printf("Features:\n");
#ifdef HAVE_LIBURING
    printf("  • io_uring async I/O for maximum performance (Linux 5.1+)\n");
#else
    printf("  • Linux AIO for asynchronous I/O\n");
#endif
    printf("  • Multi-threaded file discovery\n\n");
    printf("Options:\n");
    printf("  -r, --read-size-kb=SIZE   Size of each read request in KB (default: %d).\n", DEFAULT_READ_SIZE_KB);
    printf("  -q, --queue-depth=NUM     Number of concurrent AIO requests (default: %d).\n", DEFAULT_QUEUE_DEPTH);
    printf("  -l, --syslog              Log output to syslog.\n");
    printf("      --silent              Suppress progress output to stderr.\n");
    printf("  -d, --debug               Enable verbose debug logging.\n");
    printf("  -D, --max-depth=NUM       Limit recursion depth (default: unlimited, -1)\n");
    printf("  -T, --threads=NUM         Number of threads for discovery (default: 1, max 16)\n");
    printf("  -P, --throttle=LEVEL      Throttle I/O and CPU (0=none/default, 1-7=low to high)\n");
    printf("  -h, --help                Display this help and exit.\n");
    printf("  -v, --version             Output version information and exit.\n");
}

int config_parse_args(struct config *cfg, int argc, char **argv)
{
    struct option long_options[] = {
        {"read-size-kb", required_argument, 0, 'r'},
        {"queue-depth", required_argument, 0, 'q'},
        {"syslog", no_argument, 0, 'l'},
        {"silent", no_argument, &cfg->silent_mode, 1},
        {"debug", no_argument, 0, 'd'},
        {"help", no_argument, 0, 'h'},
        {"version", no_argument, 0, 'v'},
        {"max-depth", required_argument, 0, 'D'},
        {"threads", required_argument, 0, 'T'},
        {"throttle", required_argument, 0, 'P'},
        {0, 0, 0, 0}};

    int opt;
    while ((opt = getopt_long(argc, argv, "r:q:ldhvD:T:P:", long_options, NULL)) != -1)
    {
        switch (opt)
        {
        case 0:
            break;
        case 'r':
            cfg->read_size_kb = atol(optarg);
            break;
        case 'q':
            cfg->queue_depth = atoi(optarg);
            break;
        case 'l':
            cfg->syslog_mode = 1;
            break;
        case 'd':
            cfg->debug_mode = 1;
            break;
        case 'D':
            cfg->max_depth = atoi(optarg);
            break;
        case 'T':
            cfg->num_threads = atoi(optarg);
            if (cfg->num_threads < 1)
                cfg->num_threads = 1;
            if (cfg->num_threads > 16)
                cfg->num_threads = 16; // Arbitrary limit
            break;
        case 'P':
            cfg->throttle = atol(optarg);
            if (cfg->throttle < 0)
                cfg->throttle = 0;
            if (cfg->throttle > 7)
                cfg->throttle = 7;
            break;
        case 'h':
            config_print_help();
            return 0;
        case 'v':
            printf("page-cache-warmer version %s\n", PAGE_CACHE_WARMER_VERSION);
            return 0;
        default:
            config_print_help();
            return -1;
        }
    }

    if (optind >= argc)
    {
        fprintf(stderr, "Error: At least one <directory> argument is required.\n\n");
        config_print_help();
        return -1;
    }

    // All remaining arguments are directories
    cfg->num_directories = argc - optind;
    cfg->directories = (const char **)&argv[optind];

    return 1; // Success
}

// Utility functions
void progress_print(const char *phase_name, unsigned long long current, unsigned long long total)
{
    time_t now = time(NULL);
    if (now - g_last_log_time < 1 && current < total)
        return;
    g_last_log_time = now;

    float percentage = total > 0 ? ((float)current / total) * 100.0f : 100.0f;
    fprintf(stderr, "\r\033[2K%s: %llu / %llu files (%.2f%%)",
            phase_name, current, total, percentage);
    fflush(stderr);
}

double timing_get_duration(struct timeval start, struct timeval end)
{
    return (end.tv_sec - start.tv_sec) + (end.tv_usec - start.tv_usec) / 1000000.0;
}

void timing_print_phase(const char *phase_name, double duration)
{
    printf("\n%s completed in %.2f seconds\n", phase_name, duration);
}

void logging_init(int enable_syslog)
{
    if (enable_syslog)
    {
        openlog("page-cache-warmer", LOG_PID, LOG_USER);
    }
}

void logging_cleanup(void)
{
    closelog();
}

int main(int argc, char **argv)
{
    struct config config;
    config_init(&config);

    // Parse command line arguments
    int parse_result = config_parse_args(&config, argc, argv);
    if (parse_result <= 0)
    {
        return parse_result == 0 ? 0 : 1;
    }

    // Initialize logging
    logging_init(config.syslog_mode);

    DEBUG_LOG(&config, "Configuration parsed successfully");
    DEBUG_LOG(&config, "  Read size: %ld KB", config.read_size_kb);
    DEBUG_LOG(&config, "  Queue depth: %d", config.queue_depth);
    DEBUG_LOG(&config, "  Number of directories: %d", config.num_directories);
    DEBUG_LOG(&config, "  Max depth: %d", config.max_depth);
    DEBUG_LOG(&config, "  Number of threads: %d", config.num_threads);
    DEBUG_LOG(&config, "  Throttle: %d", config.throttle);

    if (config.syslog_mode)
    {
        syslog(LOG_INFO, "Starting page cache warming for %d directories",
               config.num_directories);
    }

    // Set priorities if throttling is enabled
    int orig_nice = 0;
#ifdef __linux__
    int orig_ioprio = 0;
#endif
    if (config.throttle > 0)
    {
        orig_nice = getpriority(PRIO_PROCESS, 0);
#ifdef __linux__
        orig_ioprio = syscall(SYS_ioprio_get, 1, 0);
#endif

        int nice_val = 10 + config.throttle;
        setpriority(PRIO_PROCESS, 0, nice_val);

#ifdef __linux__
        int ioclass = (config.throttle >= 4) ? 3 : 2; // Idle for high throttle
        int iolevel = (ioclass == 3) ? 0 : config.throttle + 3;
        if (iolevel > 7) iolevel = 7;
        int ioprio = (ioclass << 13) | iolevel;
        syscall(SYS_ioprio_set, 1, 0, ioprio);
        DEBUG_LOG(&config, "Applied throttling: nice=%d, ioprio=0x%x", nice_val, ioprio);
#else
        DEBUG_LOG(&config, "Applied throttling: nice=%d", nice_val);
#endif
    }

    struct timeval start_time, end_time;
    gettimeofday(&start_time, NULL);

    if (!config.silent_mode)
        printf("=== Discovering files... ===\n");

    struct file_list files;
    file_list_init(&files);

    for (int i = 0; i < config.num_directories; i++)
    {
        discover_files(config.directories[i], &files, 0, config.max_depth, config.num_threads);
    }

    if (!config.silent_mode)
        printf("Found %zu files to warm.\n", files.count);

    if (files.count > 0)
    {
#ifdef __linux__
#ifdef HAVE_LIBURING
        if (!config.silent_mode)
            printf("Using io_uring for asynchronous I/O\n");
        io_warm_files_uring(&files, config.read_size_kb * 1024, config.queue_depth, "Warming files", &config);
#else
        if (!config.silent_mode)
            printf("Using libaio for asynchronous I/O\n");
        io_warm_files(&files, config.read_size_kb * 1024, config.queue_depth, "Warming files", &config);
#endif
#else
    if (!config.silent_mode)
        printf("Skipping I/O warming: not supported on this platform.\n");
#endif
    }

    file_list_free(&files);

    gettimeofday(&end_time, NULL);
    timing_print_phase("Total warming time", timing_get_duration(start_time, end_time));

    // Restore original priorities
    if (config.throttle > 0)
    {
        setpriority(PRIO_PROCESS, 0, orig_nice);
#ifdef __linux__
        syscall(SYS_ioprio_set, 1, 0, orig_ioprio);
#endif
        DEBUG_LOG(&config, "Restored original priorities");
    }
    
    logging_cleanup();

    return 0;
}