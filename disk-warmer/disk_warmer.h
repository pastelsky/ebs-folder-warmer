#ifndef DISK_WARMER_H
#define DISK_WARMER_H

#define _GNU_SOURCE

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <dirent.h>
#include <sys/types.h>
#include <sys/stat.h>
#include <fcntl.h>
#include <unistd.h>
#include <sys/ioctl.h>
#include <linux/fs.h>
#include <linux/fiemap.h>
#include <limits.h>
#include <errno.h>
#include <linux/types.h>
#include <libaio.h>
#include <getopt.h>
#include <syslog.h>
#include <time.h>
#include <sys/time.h>
#include <signal.h>

#ifdef HAVE_LIBURING
#include <liburing.h>
#endif

// Additional includes for block device ioctls
#ifndef BLKSSZGET
#define BLKSSZGET _IO(0x12, 104)
#endif
#ifndef BLKPBSZGET
#define BLKPBSZGET _IO(0x12, 123)
#endif

// Constants
#define DISK_WARMER_VERSION "1.4.0"
#define LOG_INTERVAL_SECONDS 5
#define DEFAULT_READ_SIZE_KB 4
#define DEFAULT_STRIDE_KB 512
#define DEFAULT_QUEUE_DEPTH 128
#define DEFAULT_ALIGNMENT_BYTES 512
#define EBS_OPTIMAL_MERGE_SIZE_MB 16
#define FIEMAP_EXTENT_BATCH_SIZE 32
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

#define DEBUG_LOG_ALWAYS(cfg, fmt, ...) \
    do { \
        if ((cfg)->debug_mode) { \
            fprintf(stderr, "[DEBUG] " fmt "\n", ##__VA_ARGS__); \
        } \
    } while (0)

// Data structures
struct extent {
    off_t physical_offset;
    off_t length;
};

struct extent_list {
    struct extent *extents;
    size_t count;
    size_t capacity;
};

struct warmed_bitmap {
    unsigned char *data;
    size_t size_bytes;
    off_t block_size;
    off_t disk_size;
};

struct device_info {
    off_t size;
    int logical_sector_size;
    int physical_sector_size;
    int supports_direct_io;
};

struct config {
    long read_size_kb;
    long stride_kb;
    int queue_depth;
    int silent_mode;
    int syslog_mode;
    int full_disk_mode;
    int merge_extents_enabled;
    int num_directories;
    const char **directories;
    const char *device_path;
    int debug_mode;
};

struct timing_info {
    struct timeval start;
    struct timeval end;
};

// Global state (to be minimized)
extern time_t g_last_log_time;

// Function declarations

// Configuration
void config_init(struct config *cfg);
int config_parse_args(struct config *cfg, int argc, char **argv);
void config_print_help(void);

// Extent management
void extent_list_init(struct extent_list *list);
void extent_list_append(struct extent_list *list, off_t physical_offset, off_t length);
void extent_list_free(struct extent_list *list);
int extent_compare(const void *a, const void *b);
size_t extent_list_merge_adjacent(struct extent_list *list, long long max_merge_size);

// Bitmap operations
void bitmap_init(struct warmed_bitmap *bitmap, off_t disk_size, off_t block_size);
void bitmap_mark_range(struct warmed_bitmap *bitmap, off_t start, off_t length);
int bitmap_is_marked(struct warmed_bitmap *bitmap, off_t offset);
void bitmap_free(struct warmed_bitmap *bitmap);

// Device operations
int device_open_with_direct_io(const char *device_path, int *use_direct_io);
int device_get_info(int fd, struct device_info *info);
off_t device_get_size(int fd);
void device_align_io_params(const struct device_info *info, int use_direct_io, 
                           long long *read_size, long long *stride);

// File system operations
void filesystem_discover_extents(const char *directory_path, struct extent_list *list);
void filesystem_extract_file_extents(const char *file_path, struct extent_list *list);

// I/O warming operations
int io_warm_extents(int device_fd, const struct extent_list *list, 
                   struct warmed_bitmap *bitmap, const char *phase_name,
                   long long read_size, long long stride, int queue_depth, int debug_mode);
int io_warm_remaining_disk(int device_fd, struct warmed_bitmap *bitmap,
                          long long read_size, long long stride, int queue_depth, int debug_mode);

#ifdef HAVE_LIBURING
int io_warm_extents_uring(int device_fd, const struct extent_list *list,
                         struct warmed_bitmap *bitmap, const char *phase_name,
                         long long read_size, long long stride, int queue_depth, int debug_mode);
int io_warm_remaining_disk_uring(int device_fd, struct warmed_bitmap *bitmap,
                                long long read_size, long long stride, int queue_depth, int debug_mode);
#endif

// Utility functions
void progress_print(const char *phase_name, unsigned long long current, unsigned long long total);
double timing_get_duration(struct timeval start, struct timeval end);
void timing_print_phase(const char *phase_name, double duration);
void logging_init(int enable_syslog);
void logging_cleanup(void);

#endif // DISK_WARMER_H 