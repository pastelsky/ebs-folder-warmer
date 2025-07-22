#ifndef PAGE_CACHE_WARMER_H
#define PAGE_CACHE_WARMER_H

#define _GNU_SOURCE

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <dirent.h>
#include <sys/types.h>
#include <sys/stat.h>
#include <fcntl.h>
#include <unistd.h>
#ifdef __linux__
#include <libaio.h>
#endif
#include <getopt.h>
#include <syslog.h>
#include <time.h>
#include <sys/time.h>
#include <signal.h>
#include <pthread.h>

#ifdef HAVE_LIBURING
#include <liburing.h>
#endif

// Constants
#define PAGE_CACHE_WARMER_VERSION "1.0.0"
#define LOG_INTERVAL_SECONDS 5
#define DEFAULT_READ_SIZE_KB 128
#define DEFAULT_QUEUE_DEPTH 128
#define DEFAULT_ALIGNMENT_BYTES 512
#define MAX_PATH_LENGTH PATH_MAX

// Debug logging macros
#ifdef DEBUG
#define DEBUG_ENABLED 1
#else
#define DEBUG_ENABLED 0
#endif

#define DEBUG_LOG(cfg, fmt, ...) \
    do { \
        if ((cfg)->debug_mode && !((cfg)->silent_mode)) { \
            fprintf(stderr, "[DEBUG] " fmt "\n", ##__VA_ARGS__); \
        } \
    } while (0)

// Data structures
struct file_info {
    char *path;
    off_t size;
};

struct file_list {
    struct file_info *files;
    size_t count;
    size_t capacity;
};

struct config {
    long read_size_kb;
    int queue_depth;
    int silent_mode;
    int syslog_mode;
    int num_directories;
    const char **directories;
    int debug_mode;
    int max_depth;
    int num_threads;
    int throttle;
};

// Global state
extern time_t g_last_log_time;

// Function declarations

// Configuration
void config_init(struct config *cfg);
int config_parse_args(struct config *cfg, int argc, char **argv);
void config_print_help(void);

// File list management
void file_list_init(struct file_list *list);
void file_list_append(struct file_list *list, const char *path, off_t size);
void file_list_free(struct file_list *list);

// File system operations
void discover_files(const char *directory_path, struct file_list *list, int current_depth, int max_depth, int num_threads);

// I/O warming operations
// These are Linux-specific due to dependency on libaio/io_uring
#ifdef __linux__
int io_warm_files(struct file_list *files, long long read_size, int queue_depth, const char* phase_name, const struct config* cfg);
#ifdef HAVE_LIBURING
int io_warm_files_uring(struct file_list *files, long long read_size, int queue_depth, const char* phase_name, const struct config* cfg);
#endif
#else
// Provide a stub or alternative for non-Linux platforms if needed
#endif

// Utility functions
void progress_print(const char *phase_name, unsigned long long current, unsigned long long total);
double timing_get_duration(struct timeval start, struct timeval end);
void timing_print_phase(const char *phase_name, double duration);
void logging_init(int enable_syslog);
void logging_cleanup(void);

#endif // PAGE_CACHE_WARMER_H 