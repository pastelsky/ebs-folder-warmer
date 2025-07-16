#include "disk_warmer.h"

// Global state (minimized)
time_t g_last_log_time = 0;

// Configuration functions
void config_init(struct config *cfg)
{
    cfg->read_size_kb = DEFAULT_READ_SIZE_KB;
    cfg->stride_kb = DEFAULT_STRIDE_KB;
    cfg->queue_depth = DEFAULT_QUEUE_DEPTH;
    cfg->silent_mode = 0;
    cfg->syslog_mode = 0;
    cfg->full_disk_mode = 0;
    cfg->merge_extents_enabled = 0;
    cfg->num_directories = 0;
    cfg->directories = NULL;
    cfg->device_path = NULL;
    cfg->debug_mode = 0;
}

void config_print_help(void)
{
    printf("Usage: disk-warmer [OPTIONS] <directory1> [directory2 ...] <device>\n");
    printf("High-performance disk warming utility optimized for AWS EBS volumes and other block devices.\n");
    printf("Features:\n");
#ifdef HAVE_LIBURING
    printf("  • io_uring async I/O for maximum performance (Linux 5.1+)\n");
#else
    printf("  • Linux AIO for asynchronous I/O\n");
#endif
    printf("  • Direct I/O (O_DIRECT) bypassing page cache\n");
    printf("  • Automatic device alignment detection\n");
    printf("  • Physical extent mapping via FIEMAP\n");
    printf("  • Smart deduplication in full-disk mode\n\n");
    printf("By default, only warms the specified directories. Use --full-disk to warm entire device.\n");
    printf("Multiple directories can be specified and will be processed sequentially.\n\n");
    printf("Options:\n");
    printf("  -r, --read-size-kb=SIZE   Size of each read request in KB (default: %d).\n", DEFAULT_READ_SIZE_KB);
    printf("                            Auto-aligned to device sector size with O_DIRECT.\n");
    printf("  -s, --stride-kb=SIZE      Distance between reads in each extent in KB (default: %d).\n", DEFAULT_STRIDE_KB);
    printf("  -q, --queue-depth=NUM     Number of concurrent AIO requests (default: %d).\n", DEFAULT_QUEUE_DEPTH);
    printf("  -f, --full-disk           Warm entire disk after directories (two-phase mode).\n");
    printf("  -m, --merge-extents       Merge adjacent extents for larger sequential reads.\n");
    printf("                            Optimized for EBS volumes (limits merges to %dMB).\n", EBS_OPTIMAL_MERGE_SIZE_MB);
    printf("  -l, --syslog              Log output to syslog.\n");
    printf("      --silent              Suppress progress output to stderr.\n");
    printf("  -d, --debug               Enable verbose debug logging.\n");
    printf("  -h, --help                Display this help and exit.\n");
    printf("  -v, --version             Output version information and exit.\n");
}

int config_parse_args(struct config *cfg, int argc, char **argv)
{
    struct option long_options[] = {
        {"read-size-kb", required_argument, 0, 'r'},
        {"stride-kb", required_argument, 0, 's'},
        {"queue-depth", required_argument, 0, 'q'},
        {"full-disk", no_argument, 0, 'f'},
        {"merge-extents", no_argument, 0, 'm'},
        {"syslog", no_argument, 0, 'l'},
        {"silent", no_argument, &cfg->silent_mode, 1},
        {"debug", no_argument, 0, 'd'},
        {"help", no_argument, 0, 'h'},
        {"version", no_argument, 0, 'v'},
        {0, 0, 0, 0}};

    int opt;
    while ((opt = getopt_long(argc, argv, "r:s:q:fmldhv", long_options, NULL)) != -1)
    {
        switch (opt)
        {
        case 0:
            break;
        case 'r':
            cfg->read_size_kb = atol(optarg);
            break;
        case 's':
            cfg->stride_kb = atol(optarg);
            break;
        case 'q':
            cfg->queue_depth = atoi(optarg);
            break;
        case 'f':
            cfg->full_disk_mode = 1;
            break;
        case 'm':
            cfg->merge_extents_enabled = 1;
            break;
        case 'l':
            cfg->syslog_mode = 1;
            break;
        case 'd':
            cfg->debug_mode = 1;
            break;
        case 'h':
            config_print_help();
            return 0;
        case 'v':
            printf("disk-warmer version %s\n", DISK_WARMER_VERSION);
            return 0;
        default:
            config_print_help();
            return -1;
        }
    }

    if (optind + 2 > argc)
    {
        fprintf(stderr, "Error: At least one <directory> and <device> argument required.\n\n");
        config_print_help();
        return -1;
    }

    // Parse directories and device - device is the last argument
    cfg->num_directories = argc - optind - 1;
    cfg->directories = (const char **)&argv[optind];
    cfg->device_path = argv[argc - 1];

    return 1; // Success
}

// Extent management functions
void extent_list_init(struct extent_list *list)
{
    list->extents = NULL;
    list->count = 0;
    list->capacity = 0;
}

void extent_list_append(struct extent_list *list, off_t physical_offset, off_t length)
{
    if (list->count >= list->capacity)
    {
        size_t new_capacity = list->capacity ? list->capacity * 2 : 16;
        struct extent *new_extents = realloc(list->extents, new_capacity * sizeof(struct extent));
        if (!new_extents)
        {
            perror("realloc extent_list");
            return;
        }
        list->extents = new_extents;
        list->capacity = new_capacity;
    }
    list->extents[list->count].physical_offset = physical_offset;
    list->extents[list->count].length = length;
    list->count++;
}

void extent_list_free(struct extent_list *list)
{
    free(list->extents);
    list->extents = NULL;
    list->count = 0;
    list->capacity = 0;
}

int extent_compare(const void *a, const void *b)
{
    const struct extent *extent_a = a;
    const struct extent *extent_b = b;
    if (extent_a->physical_offset < extent_b->physical_offset)
        return -1;
    if (extent_a->physical_offset > extent_b->physical_offset)
        return 1;
    return 0;
}

size_t extent_list_merge_adjacent(struct extent_list *list, long long max_merge_size)
{
    if (list->count <= 1)
        return list->count;

    size_t write_index = 0;

    for (size_t read_index = 0; read_index < list->count; read_index++)
    {
        // Copy current extent to write position
        list->extents[write_index] = list->extents[read_index];

        // Try to merge with subsequent adjacent extents
        while (read_index + 1 < list->count)
        {
            struct extent *current = &list->extents[write_index];
            struct extent *next = &list->extents[read_index + 1];

            // Check if extents are adjacent
            if (current->physical_offset + current->length == next->physical_offset)
            {
                // Check if merged size would exceed limit (for EBS optimization)
                if (max_merge_size > 0 && current->length + next->length > max_merge_size)
                {
                    break; // Don't merge if it would exceed EBS-friendly size
                }

                // Merge the extents
                current->length += next->length;
                read_index++;

                // Log significant merges
                if (current->length > 1024 * 1024)
                {
                    static time_t last_merge_log = 0;
                    time_t now = time(NULL);
                    if (now - last_merge_log >= LOG_INTERVAL_SECONDS)
                    {
                        printf("Merged adjacent extents: %lld MB region\n",
                               current->length / (1024 * 1024));
                        last_merge_log = now;
                    }
                }
            }
            else
            {
                break; // Not adjacent
            }
        }
        write_index++;
    }

    list->count = write_index;
    return write_index;
}

// Bitmap operations
void bitmap_init(struct warmed_bitmap *bitmap, off_t disk_size, off_t block_size)
{
    bitmap->disk_size = disk_size;
    bitmap->block_size = block_size;
    size_t num_blocks = (disk_size + block_size - 1) / block_size;
    bitmap->size_bytes = (num_blocks + 7) / 8;
    bitmap->data = calloc(1, bitmap->size_bytes);
}

void bitmap_mark_range(struct warmed_bitmap *bitmap, off_t start, off_t length)
{
    if (!bitmap->data)
        return;

    off_t start_block = start / bitmap->block_size;
    off_t end_block = (start + length - 1) / bitmap->block_size;

    for (off_t block = start_block;
         block <= end_block && block < (bitmap->disk_size / bitmap->block_size);
         block++)
    {
        size_t byte_index = block / 8;
        int bit_index = block % 8;
        if (byte_index < bitmap->size_bytes)
        {
            bitmap->data[byte_index] |= (1 << bit_index);
        }
    }
}

int bitmap_is_marked(struct warmed_bitmap *bitmap, off_t offset)
{
    if (!bitmap->data)
        return 0;

    off_t block = offset / bitmap->block_size;
    size_t byte_index = block / 8;
    int bit_index = block % 8;

    if (byte_index >= bitmap->size_bytes)
        return 0;
    return (bitmap->data[byte_index] & (1 << bit_index)) != 0;
}

void bitmap_free(struct warmed_bitmap *bitmap)
{
    free(bitmap->data);
    bitmap->data = NULL;
}

// Device operations
int device_open_with_direct_io(const char *device_path, int *use_direct_io)
{
    // Try O_DIRECT first for better performance
    int fd = open(device_path, O_RDONLY | O_DIRECT);
    *use_direct_io = 1;

    if (fd == -1)
    {
        // Fallback to buffered I/O if O_DIRECT fails
        fd = open(device_path, O_RDONLY);
        *use_direct_io = 0;

        if (fd == -1)
        {
            return -1;
        }

        printf("Note: Using buffered I/O (O_DIRECT not supported)\n");
    }
    else
    {
        printf("Using direct I/O for optimal performance\n");
    }

    return fd;
}

int device_get_info(int fd, struct device_info *info)
{
    // Get device size
    info->size = lseek(fd, 0, SEEK_END);
    lseek(fd, 0, SEEK_SET);

    if (info->size <= 0)
    {
        return -1;
    }

    // Set defaults
    info->logical_sector_size = DEFAULT_ALIGNMENT_BYTES;
    info->physical_sector_size = DEFAULT_ALIGNMENT_BYTES;
    info->supports_direct_io = 1;

    // Query logical sector size
    if (ioctl(fd, BLKSSZGET, &info->logical_sector_size) != 0)
    {
        info->logical_sector_size = DEFAULT_ALIGNMENT_BYTES;
    }

    // Query physical sector size
    if (ioctl(fd, BLKPBSZGET, &info->physical_sector_size) != 0)
    {
        info->physical_sector_size = info->logical_sector_size;
    }

    return 0;
}

off_t device_get_size(int fd)
{
    struct device_info info;
    if (device_get_info(fd, &info) == 0)
    {
        return info.size;
    }
    return -1;
}

void device_align_io_params(const struct device_info *info, int use_direct_io,
                            long long *read_size, long long *stride)
{
    if (!use_direct_io)
    {
        return; // No alignment needed for buffered I/O
    }

    int alignment = info->physical_sector_size;

    // Align read size
    if (*read_size % alignment != 0)
    {
        *read_size = ((*read_size + alignment - 1) / alignment) * alignment;
        printf("Adjusted read size to %lld bytes for %d-byte sector alignment\n",
               *read_size, alignment);
    }

    // Align stride
    if (*stride % alignment != 0)
    {
        *stride = ((*stride + alignment - 1) / alignment) * alignment;
        printf("Adjusted stride to %lld bytes for sector alignment\n", *stride);
    }
}

// File system operations
void filesystem_extract_file_extents(const char *file_path, struct extent_list *list)
{
    int fd = open(file_path, O_RDONLY);
    if (fd == -1)
    {
      perror(file_path);
      return;
  }

  struct stat st;
  if (fstat(fd, &st) == -1) {
    close(fd);
    return;
  }

  if (st.st_size == 0) {
    close(fd);
    return;
  }

  size_t fiemap_size = sizeof(struct fiemap) +
                       (FIEMAP_EXTENT_BATCH_SIZE * sizeof(struct fiemap_extent));
  struct fiemap *fiemap = calloc(1, fiemap_size);
  if (!fiemap)
  {
      close(fd);
      return;
  }

  __u64 offset = 0;
  do {
      fiemap->fm_start = offset;
      fiemap->fm_length = (~(__u64)0ULL) - offset;
      fiemap->fm_flags = FIEMAP_FLAG_SYNC;
      fiemap->fm_extent_count = FIEMAP_EXTENT_BATCH_SIZE;
      fiemap->fm_mapped_extents = 0;

      if (ioctl(fd, FS_IOC_FIEMAP, fiemap) == -1)
      {
          perror("FIEMAP");
          break;
      }

    if (fiemap->fm_mapped_extents == 0)
        break;

    for (unsigned i = 0; i < fiemap->fm_mapped_extents; i++)
    {
        struct fiemap_extent *ext = &fiemap->fm_extents[i];
        if (ext->fe_flags & FIEMAP_EXTENT_UNKNOWN)
            continue;

        extent_list_append(list, ext->fe_physical, ext->fe_length);
        offset = ext->fe_logical + ext->fe_length;

        if (ext->fe_flags & FIEMAP_EXTENT_LAST)
        {
            offset = 0;
        }
    }
  } while (offset > 0);

  free(fiemap);
  close(fd);
}

void filesystem_discover_extents(const char *directory_path, struct extent_list *list)
{
    DIR *dir = opendir(directory_path);
    if (!dir)
        return;

  struct dirent *entry;
  while ((entry = readdir(dir))) {
      if (strcmp(entry->d_name, ".") == 0 || strcmp(entry->d_name, "..") == 0)
          continue;

      char path[MAX_PATH_LENGTH];
      snprintf(path, sizeof(path), "%s/%s", directory_path, entry->d_name);

      struct stat st;
      if (lstat(path, &st) < 0)
      {
          perror(path);
          continue;
    }

    if (S_ISDIR(st.st_mode)) {
        filesystem_discover_extents(path, list);
    } else if (S_ISREG(st.st_mode)) {
        filesystem_extract_file_extents(path, list);
    } else if (S_ISLNK(st.st_mode)) {
        char target_path[MAX_PATH_LENGTH];
        ssize_t len = readlink(path, target_path, sizeof(target_path) - 1);
        if (len != -1)
        {
            target_path[len] = '\0';

            char final_path[MAX_PATH_LENGTH];
            if (target_path[0] == '/')
            {
                strncpy(final_path, target_path, sizeof(final_path));
            }
            else
            {
                snprintf(final_path, sizeof(final_path), "%s/%s",
                         directory_path, target_path);
            }

            if (stat(final_path, &st) == 0 && S_ISREG(st.st_mode))
            {
                filesystem_extract_file_extents(final_path, list);
            }
        }
    }
  }
  closedir(dir);
}

// Utility functions
void progress_print(const char *phase_name, unsigned long long current, unsigned long long total)
{
    time_t now = time(NULL);
    if (now - g_last_log_time < 1 && current < total)
        return;
    g_last_log_time = now;

    float percentage = total > 0 ? ((float)current / total) * 100.0f : 100.0f;
    fprintf(stderr, "\r\033[2K%s: %llu / %llu (%.2f%%)",
            phase_name, current, total, percentage);
    fflush(stderr);
}

double timing_get_duration(struct timeval start, struct timeval end)
{
    return (end.tv_sec - start.tv_sec) + (end.tv_usec - start.tv_usec) / 1000000.0;
}

void timing_print_phase(const char *phase_name, double duration)
{
    printf("%s completed in %.2f seconds\n", phase_name, duration);
}

void logging_init(int enable_syslog)
{
    if (enable_syslog)
    {
        openlog("disk-warmer", LOG_PID, LOG_USER);
    }
}

void logging_cleanup(void)
{
    closelog();
}

// Main execution
static int execute_directory_warming_phase(const struct config *cfg, int device_fd,
                                           struct extent_list *extent_list,
                                           struct warmed_bitmap *bitmap,
                                           long long read_size, long long stride,
                                           struct timeval *phase_start, struct timeval *phase_end)
{
    if (!cfg->silent_mode)
        printf("=== Phase 1: Discovering and warming directory files ===\n");
    DEBUG_LOG(cfg, "Starting Phase 1: directory discovery and warming");
    DEBUG_LOG(cfg, "Phase 1 parameters: read_size=%lld, stride=%lld, device_fd=%d",
              read_size, stride, device_fd);
    gettimeofday(phase_start, NULL);

    // Process all specified directories
    for (int i = 0; i < cfg->num_directories; i++)
    {
        if (!cfg->silent_mode && cfg->num_directories > 1)
            printf("Processing directory %d/%d: %s\n", i + 1, cfg->num_directories, cfg->directories[i]);
        if (cfg->syslog_mode)
            syslog(LOG_INFO, "Processing directory: %s", cfg->directories[i]);
        DEBUG_LOG(cfg, "Discovering extents in directory: %s", cfg->directories[i]);
        size_t extents_before = extent_list->count;
        filesystem_discover_extents(cfg->directories[i], extent_list);
        DEBUG_LOG(cfg, "Directory %s added %zu extents (total now: %zu)",
                  cfg->directories[i], extent_list->count - extents_before, extent_list->count);
    }

    if (!cfg->silent_mode)
        printf("Found %zu extents across %d directories to warm.\n",
               extent_list->count, cfg->num_directories);

    if (extent_list->count == 0)
    {
        if (!cfg->silent_mode)
            printf("No files found in specified directories.\n");
        if (cfg->syslog_mode)
            syslog(LOG_INFO, "No files found in specified directories.");
        gettimeofday(phase_end, NULL);
        timing_print_phase("Phase 1 (discovery only)",
                           timing_get_duration(*phase_start, *phase_end));
        return 0;
    }

    // Sort extents for sequential reading
    DEBUG_LOG(cfg, "Sorting %zu extents for sequential reading", extent_list->count);
    qsort(extent_list->extents, extent_list->count, sizeof(struct extent), extent_compare);
    if (!cfg->silent_mode)
        printf("Directory extents sorted for sequential reading.\n");
    DEBUG_LOG(cfg, "Extents sorted successfully");

    // Merge adjacent extents if requested
    if (cfg->merge_extents_enabled)
    {
        size_t original_count = extent_list->count;
        DEBUG_LOG(cfg, "Starting extent merging with %zu extents (max merge size: %d MB)",
                  original_count, EBS_OPTIMAL_MERGE_SIZE_MB);
        size_t merged_count = extent_list_merge_adjacent(extent_list,
                                                         EBS_OPTIMAL_MERGE_SIZE_MB * 1024 * 1024);
        DEBUG_LOG(cfg, "Extent merging completed: %zu -> %zu extents", original_count, merged_count);
        if (!cfg->silent_mode && merged_count < original_count)
        {
            printf("Merged %zu extents into %zu larger sequential regions\n",
                   original_count, merged_count);
        }
        if (cfg->syslog_mode)
        {
            syslog(LOG_INFO, "Merged %zu extents into %zu regions",
                   original_count, merged_count);
        }
    }
    else
    {
        DEBUG_LOG(cfg, "Extent merging disabled");
    }

    if (cfg->syslog_mode)
        syslog(LOG_INFO, "Found %zu extents across %d directories to warm.",
               extent_list->count, cfg->num_directories);

    // Perform the actual warming
#ifdef HAVE_LIBURING
    if (!cfg->silent_mode)
    {
        printf("Using io_uring for asynchronous I/O\n");
    }
    if (io_warm_extents_uring(device_fd, extent_list, bitmap,
                              "Phase 1 - Directory files", read_size, stride, cfg->queue_depth, cfg->debug_mode) < 0)
#else
    if (!cfg->silent_mode)
    {
        printf("Using libaio for asynchronous I/O\n");
    }
    if (io_warm_extents(device_fd, extent_list, bitmap,
                        "Phase 1 - Directory files", read_size, stride, cfg->queue_depth, cfg->debug_mode) < 0)
#endif
    {
        return -1;
    }

    gettimeofday(phase_end, NULL);
    timing_print_phase("Phase 1 (directory warming)",
                       timing_get_duration(*phase_start, *phase_end));
    return 0;
}

static int execute_full_disk_warming_phase(const struct config *cfg, int device_fd,
                                           struct warmed_bitmap *bitmap,
                                           long long read_size, long long stride,
                                           struct timeval *phase_start, struct timeval *phase_end)
{
    if (!cfg->silent_mode)
        printf("\n=== Phase 2: Warming remaining disk blocks ===\n");
    if (cfg->syslog_mode)
        syslog(LOG_INFO, "Starting phase 2: warming remaining disk blocks");
    gettimeofday(phase_start, NULL);

#ifdef HAVE_LIBURING
    if (io_warm_remaining_disk_uring(device_fd, bitmap, read_size, stride, cfg->queue_depth, cfg->debug_mode) < 0)
#else
    if (io_warm_remaining_disk(device_fd, bitmap, read_size, stride, cfg->queue_depth, cfg->debug_mode) < 0)
#endif
    {
        return -1;
    }

    gettimeofday(phase_end, NULL);
    timing_print_phase("Phase 2 (remaining disk warming)",
                       timing_get_duration(*phase_start, *phase_end));
    return 0;
}

int main(int argc, char **argv)
{
    struct config config;
    config_init(&config);

    // Parse command line arguments
    int parse_result = config_parse_args(&config, argc, argv);
    if (parse_result <= 0)
    {
        return parse_result == 0 ? 0 : 1; // 0 = help/version, -1 = error
    }

    // Initialize logging
    logging_init(config.syslog_mode);

    DEBUG_LOG(&config, "Configuration parsed successfully");
    DEBUG_LOG(&config, "  Read size: %ld KB", config.read_size_kb);
    DEBUG_LOG(&config, "  Stride: %ld KB", config.stride_kb);
    DEBUG_LOG(&config, "  Queue depth: %d", config.queue_depth);
    DEBUG_LOG(&config, "  Number of directories: %d", config.num_directories);
    DEBUG_LOG(&config, "  Device: %s", config.device_path);
    DEBUG_LOG(&config, "  Full disk mode: %s", config.full_disk_mode ? "enabled" : "disabled");
    DEBUG_LOG(&config, "  Merge extents: %s", config.merge_extents_enabled ? "enabled" : "disabled");

    if (config.syslog_mode)
    {
        if (config.full_disk_mode)
        {
            syslog(LOG_INFO, "Starting two-phase warmup for %d directories on device '%s'",
                   config.num_directories, config.device_path);
        }
        else
        {
            syslog(LOG_INFO, "Starting directory warmup for %d directories on device '%s'",
                   config.num_directories, config.device_path);
        }
    }

    // Validate device
    DEBUG_LOG(&config, "Validating device: %s", config.device_path);
    struct stat stat_buf;
    if (stat(config.device_path, &stat_buf) != 0)
    {
        perror("stat device");
        if (config.syslog_mode)
            syslog(LOG_ERR, "Failed to stat device %s: %m", config.device_path);
        return 1;
    }

    DEBUG_LOG(&config, "Device stat successful - mode: 0%o, size: %ld",
              stat_buf.st_mode, stat_buf.st_size);

    if (!S_ISBLK(stat_buf.st_mode))
    {
        fprintf(stderr, "Warning: Device %s is not a block device. Continuing anyway.\n",
                config.device_path);
        DEBUG_LOG(&config, "Device is not a block device (mode: 0%o)", stat_buf.st_mode);
        if (config.syslog_mode)
            syslog(LOG_WARNING, "Device %s is not a block device.", config.device_path);
    }
    else
    {
        DEBUG_LOG(&config, "Device is a valid block device");
    }

    // Open device
    DEBUG_LOG(&config, "Opening device for I/O");
    int use_direct_io;
    int device_fd = device_open_with_direct_io(config.device_path, &use_direct_io);
    if (device_fd == -1)
    {
        perror("open device");
        return 1;
    }
    DEBUG_LOG(&config, "Device opened successfully (fd=%d, direct_io=%s)",
              device_fd, use_direct_io ? "enabled" : "disabled");

    // Get device information
    DEBUG_LOG(&config, "Querying device information");
    struct device_info device_info;
    if (device_get_info(device_fd, &device_info) != 0)
    {
        fprintf(stderr, "Failed to get device information\n");
        close(device_fd);
        return 1;
    }
    DEBUG_LOG(&config, "Device info: size=%ld bytes, logical_sector=%d, physical_sector=%d",
              device_info.size, device_info.logical_sector_size, device_info.physical_sector_size);

    // Calculate aligned I/O parameters
    long long read_size = config.read_size_kb * 1024;
    long long stride = config.stride_kb * 1024;
    DEBUG_LOG(&config, "Initial I/O parameters: read_size=%lld, stride=%lld", read_size, stride);
    device_align_io_params(&device_info, use_direct_io, &read_size, &stride);
    DEBUG_LOG(&config, "Aligned I/O parameters: read_size=%lld, stride=%lld", read_size, stride);

    // Initialize bitmap for tracking warmed blocks
    struct warmed_bitmap bitmap;
    bitmap_init(&bitmap, device_info.size, stride);

    // Initialize extent list
    struct extent_list extent_list;
    extent_list_init(&extent_list);

    // Timing variables
    struct timeval overall_start, phase1_start, phase1_end, phase2_start, phase2_end;
    gettimeofday(&overall_start, NULL);

    // Phase 1: Directory warming
    int result = execute_directory_warming_phase(&config, device_fd, &extent_list, &bitmap,
                                                 read_size, stride, &phase1_start, &phase1_end);
    if (result < 0)
    {
        extent_list_free(&extent_list);
        bitmap_free(&bitmap);
        close(device_fd);
        return 1;
    }

    // Phase 2: Full disk warming (optional)
    if (config.full_disk_mode)
    {
        result = execute_full_disk_warming_phase(&config, device_fd, &bitmap,
                                                 read_size, stride, &phase2_start, &phase2_end);
        if (result < 0)
        {
            extent_list_free(&extent_list);
            bitmap_free(&bitmap);
            close(device_fd);
            return 1;
        }

        double total_time = timing_get_duration(overall_start, phase2_end);
        if (!config.silent_mode)
            printf("\n=== Two-phase disk warming completed successfully ===\n");
        timing_print_phase("Total warming time", total_time);
    }
    else
    {
        double total_time = timing_get_duration(overall_start, phase1_end);
        if (!config.silent_mode)
            printf("\n=== Directory warming completed successfully ===\n");
        timing_print_phase("Total warming time", total_time);
    }

    // Log completion
    if (config.syslog_mode)
    {
        if (config.full_disk_mode)
        {
            syslog(LOG_INFO, "Two-phase disk warming completed successfully.");
        }
        else
        {
            syslog(LOG_INFO, "Directory warming completed successfully.");
        }
    }

    // Cleanup
    extent_list_free(&extent_list);
    bitmap_free(&bitmap);
    close(device_fd);
    logging_cleanup();

    return 0;
}