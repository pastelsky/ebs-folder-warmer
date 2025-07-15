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

#define _GNU_SOURCE

#define DISK_WARMER_VERSION "1.2.0"
#define LOG_INTERVAL 5 // Seconds

static int silent_mode = 0;
static int syslog_mode = 0;
static int full_disk_mode = 0;
static time_t last_log_time = 0;

struct extent {
  off_t phys;
  off_t len;
};

struct extent_list {
  struct extent *data;
  size_t size;
  size_t cap;
};

// Bitmap to track warmed blocks
struct warmed_bitmap
{
    unsigned char *data;
    size_t size_bytes;
    off_t block_size;
    off_t disk_size;
};

void bitmap_init(struct warmed_bitmap *bm, off_t disk_size, off_t block_size)
{
    bm->disk_size = disk_size;
    bm->block_size = block_size;
    size_t num_blocks = (disk_size + block_size - 1) / block_size;
    bm->size_bytes = (num_blocks + 7) / 8;
    bm->data = calloc(1, bm->size_bytes);
}

void bitmap_mark_range(struct warmed_bitmap *bm, off_t start, off_t length)
{
    if (!bm->data)
        return;

    off_t start_block = start / bm->block_size;
    off_t end_block = (start + length - 1) / bm->block_size;

    for (off_t block = start_block; block <= end_block && block < (bm->disk_size / bm->block_size); block++)
    {
        size_t byte_idx = block / 8;
        int bit_idx = block % 8;
        if (byte_idx < bm->size_bytes)
        {
            bm->data[byte_idx] |= (1 << bit_idx);
        }
    }
}

int bitmap_is_marked(struct warmed_bitmap *bm, off_t offset)
{
    if (!bm->data)
        return 0;

    off_t block = offset / bm->block_size;
    size_t byte_idx = block / 8;
    int bit_idx = block % 8;

    if (byte_idx >= bm->size_bytes)
        return 0;
    return (bm->data[byte_idx] & (1 << bit_idx)) != 0;
}

void bitmap_free(struct warmed_bitmap *bm)
{
    free(bm->data);
    bm->data = NULL;
}

void extent_list_init(struct extent_list *el) {
  el->data = NULL;
  el->size = 0;
  el->cap = 0;
}

void extent_list_append(struct extent_list *el, off_t phys, off_t len) {
  if (el->size >= el->cap) {
    size_t new_cap = el->cap ? el->cap * 2 : 16;
    struct extent *new_data = realloc(el->data, new_cap * sizeof(struct extent));
    if (!new_data) {
      perror("realloc extent_list");
      return; // Or exit, depending on desired strictness
    }
    el->data = new_data;
    el->cap = new_cap;
  }
  el->data[el->size].phys = phys;
  el->data[el->size].len = len;
  el->size++;
}

void extent_list_free(struct extent_list *el) {
  free(el->data);
  el->data = NULL;
  el->size = 0;
  el->cap = 0;
}

void warm_file(const char* filename, struct extent_list *el) {
  int fd = open(filename, O_RDONLY);
  if (fd == -1) {
    perror(filename);
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
  const unsigned int extent_count = 32;
  size_t fm_size = sizeof(struct fiemap) + (extent_count * sizeof(struct fiemap_extent));
  struct fiemap *fm = calloc(1, fm_size);
  if (!fm) {
    close(fd);
    return;
  }
  __u64 offset = 0;
  do {
    fm->fm_start = offset;
    fm->fm_length = (~(__u64)0ULL) - offset;
    fm->fm_flags = FIEMAP_FLAG_SYNC;
    fm->fm_extent_count = extent_count;
    fm->fm_mapped_extents = 0;
    if (ioctl(fd, FS_IOC_FIEMAP, fm) == -1) {
      perror("FIEMAP");
      break;
    }
    if (fm->fm_mapped_extents == 0) break;
    for (unsigned i = 0; i < fm->fm_mapped_extents; i++) {
      struct fiemap_extent *ext = &fm->fm_extents[i];
      if (ext->fe_flags & FIEMAP_EXTENT_UNKNOWN) continue;
      extent_list_append(el, ext->fe_physical, ext->fe_length);
      offset = ext->fe_logical + ext->fe_length;
      if (ext->fe_flags & FIEMAP_EXTENT_LAST) {
        offset = 0;
      }
    }
  } while (offset > 0);
  free(fm);
  close(fd);
}

void traverse_dir(const char* dirpath, struct extent_list *el) {
  DIR* dir = opendir(dirpath);
  if (!dir) return;
  struct dirent* entry;
  while ((entry = readdir(dir))) {
    if (strcmp(entry->d_name, ".") == 0 || strcmp(entry->d_name, "..") == 0) continue;
    char path[PATH_MAX];
    snprintf(path, sizeof(path), "%s/%s", dirpath, entry->d_name);
    struct stat st;
    if (lstat(path, &st) < 0) {
      perror(path);
      continue;
    }

    if (S_ISDIR(st.st_mode)) {
      traverse_dir(path, el);
    } else if (S_ISREG(st.st_mode)) {
      warm_file(path, el);
    } else if (S_ISLNK(st.st_mode)) {
      char target_path[PATH_MAX];
      ssize_t len = readlink(path, target_path, sizeof(target_path) - 1);
      if (len != -1) {
        target_path[len] = '\0';
        // Handle relative symlinks by checking if the target is absolute
        char final_path[PATH_MAX];
        if (target_path[0] == '/') {
          strncpy(final_path, target_path, sizeof(final_path));
        } else {
          snprintf(final_path, sizeof(final_path), "%s/%s", dirpath, target_path);
        }

        if (stat(final_path, &st) == 0 && S_ISREG(st.st_mode)) {
          warm_file(final_path, el);
        }
      }
    }
  }
  closedir(dir);
}

int compare_extents(const void *a, const void *b) {
    const struct extent *ext_a = a;
    const struct extent *ext_b = b;
    if (ext_a->phys < ext_b->phys) return -1;
    if (ext_a->phys > ext_b->phys) return 1;
    return 0;
}

void print_progress(const char *phase, unsigned long long current, unsigned long long total)
{
    if (silent_mode) return;

    time_t now = time(NULL);
    if (now - last_log_time < 1 && current < total) return;
    last_log_time = now;

    float percentage = total > 0 ? ((float)current / total) * 100.0f : 100.0f;
    fprintf(stderr, "\r\033[2K%s: %llu / %llu (%.2f%%)", phase, current, total, percentage);
    fflush(stderr);

    if (syslog_mode) {
        syslog(LOG_INFO, "%s: %llu / %llu (%.2f%%)", phase, current, total, percentage);
    }
}

off_t get_device_size(int fd)
{
    off_t size = lseek(fd, 0, SEEK_END);
    lseek(fd, 0, SEEK_SET);
    return size;
}

int warm_extents(int dev_fd, struct extent_list *el, struct warmed_bitmap *bm,
                 const char *phase_name, long long read_size, long long stride, int max_aio)
{
    io_context_t ctx = {0};
    if (io_setup(max_aio, &ctx) < 0)
    {
        perror("io_setup");
        return -1;
    }

    char *bufs[max_aio];
    for (int i = 0; i < max_aio; i++)
    {
        if (posix_memalign((void **)&bufs[i], 512, read_size) != 0)
        {
            perror("posix_memalign for buf");
            for (int j = 0; j < i; j++)
                free(bufs[j]);
            io_destroy(ctx);
            return -1;
        }
    }

    struct iocb iocbs[max_aio];
    struct iocb *iocbsp[max_aio];
    struct io_event events[max_aio];

    size_t ext_idx = 0;
    off_t ext_off = 0;
    unsigned long long total_reads = 0;
    unsigned long long total_strides = 0;

    // Calculate total strides for progress
    for (size_t i = 0; i < el->size; ++i)
    {
        total_strides += (el->data[i].len + stride - 1) / stride;
    }

    while (ext_idx < el->size)
    {
        int n = 0;
        while (n < max_aio && ext_idx < el->size)
        {
            struct extent *e = &el->data[ext_idx];
            off_t rem = e->len - ext_off;
            if (rem <= 0)
            {
                ext_idx++;
                ext_off = 0;
                continue;
            }
            off_t this_off = e->phys + ext_off;
            io_prep_pread(&iocbs[n], dev_fd, bufs[n], read_size, this_off);
            iocbsp[n] = &iocbs[n];
            n++;
            total_reads++;

            // Mark this range as warmed
            if (bm)
            {
                bitmap_mark_range(bm, this_off, read_size);
            }

            ext_off += stride;
            if (ext_off >= e->len)
            {
                ext_idx++;
                ext_off = 0;
            }
        }
        if (n == 0)
            break;
        print_progress(phase_name, total_reads, total_strides);

        int sub = io_submit(ctx, n, iocbsp);
        if (sub != n)
        {
            if (sub < 0)
            {
                perror("io_submit");
            }
            else
            {
                fprintf(stderr, "io_submit: submitted only %d of %d\n", sub, n);
            }
            break;
        }
        int comp = io_getevents(ctx, n, n, events, NULL);
        if (comp != n)
        {
            if (comp < 0)
            {
                perror("io_getevents");
            }
            else
            {
                fprintf(stderr, "io_getevents: received only %d of %d\n", comp, n);
            }
            break;
        }
    }

    print_progress(phase_name, total_strides, total_strides); // Final update to 100%
    if (!silent_mode)
        printf("\n");
    if (syslog_mode)
    {
        syslog(LOG_INFO, "%s completed %llu reads.", phase_name, total_reads);
    }

    io_destroy(ctx);
    for (int i = 0; i < max_aio; i++)
        free(bufs[i]);
    return 0;
}

int warm_remaining_disk(int dev_fd, struct warmed_bitmap *bm, long long read_size,
                        long long stride, int max_aio)
{
    off_t disk_size = get_device_size(dev_fd);
    if (disk_size <= 0)
    {
        fprintf(stderr, "Could not determine device size\n");
        return -1;
    }

    io_context_t ctx = {0};
    if (io_setup(max_aio, &ctx) < 0)
    {
        perror("io_setup");
        return -1;
    }

    char *bufs[max_aio];
    for (int i = 0; i < max_aio; i++)
    {
        if (posix_memalign((void **)&bufs[i], 512, read_size) != 0)
        {
            perror("posix_memalign for buf");
            for (int j = 0; j < i; j++)
                free(bufs[j]);
            io_destroy(ctx);
            return -1;
        }
    }

    struct iocb iocbs[max_aio];
    struct iocb *iocbsp[max_aio];
    struct io_event events[max_aio];

    unsigned long long total_reads = 0;
    unsigned long long total_strides = (disk_size + stride - 1) / stride;
    off_t current_offset = 0;

    while (current_offset < disk_size)
    {
        int n = 0;
        while (n < max_aio && current_offset < disk_size)
        {
            // Skip if this block was already warmed
            if (bitmap_is_marked(bm, current_offset))
            {
                current_offset += stride;
                continue;
            }

            off_t read_length = read_size;
            if (current_offset + read_length > disk_size)
            {
                read_length = disk_size - current_offset;
            }

            io_prep_pread(&iocbs[n], dev_fd, bufs[n], read_length, current_offset);
            iocbsp[n] = &iocbs[n];
            n++;
            total_reads++;
            current_offset += stride;
        }

        if (n == 0)
            break;
        print_progress("Phase 2 - Remaining disk", total_reads, total_strides);

        int sub = io_submit(ctx, n, iocbsp);
        if (sub != n)
        {
            if (sub < 0)
            {
                perror("io_submit");
            }
            else
            {
                fprintf(stderr, "io_submit: submitted only %d of %d\n", sub, n);
            }
            break;
        }
        int comp = io_getevents(ctx, n, n, events, NULL);
        if (comp != n)
        {
            if (comp < 0)
            {
                perror("io_getevents");
            }
            else
            {
                fprintf(stderr, "io_getevents: received only %d of %d\n", comp, n);
            }
            break;
        }
    }

    print_progress("Phase 2 - Remaining disk", total_strides, total_strides);
    if (!silent_mode)
        printf("\n");
    if (syslog_mode)
    {
        syslog(LOG_INFO, "Phase 2 completed %llu reads.", total_reads);
    }

    io_destroy(ctx);
    for (int i = 0; i < max_aio; i++)
        free(bufs[i]);
    return 0;
}

double get_time_diff(struct timeval start, struct timeval end)
{
    return (end.tv_sec - start.tv_sec) + (end.tv_usec - start.tv_usec) / 1000000.0;
}

void print_timing(const char *phase_name, double duration)
{
    if (!silent_mode)
    {
        printf("%s completed in %.2f seconds\n", phase_name, duration);
    }
    if (syslog_mode)
    {
        syslog(LOG_INFO, "%s completed in %.2f seconds", phase_name, duration);
    }
}

void help() {
    printf("Usage: disk-warmer [OPTIONS] <directory> <device>\n");
    printf("Selectively warms up an EBS volume by reading files in a specific directory.\n");
    printf("By default, only warms the target directory. Use --full-disk to warm entire device.\n\n");
    printf("Options:\n");
    printf("  -r, --read-size-kb=SIZE   Size of each read request in KB (default: 4).\n");
    printf("  -s, --stride-kb=SIZE      Distance between reads in each extent in KB (default: 512).\n");
    printf("  -q, --queue-depth=NUM     Number of concurrent AIO requests (default: 128).\n");
    printf("  -f, --full-disk           Warm entire disk after directory (two-phase mode).\n");
    printf("  -l, --syslog              Log output to syslog.\n");
    printf("      --silent              Suppress progress output to stderr.\n");
    printf("  -h, --help                Display this help and exit.\n");
    printf("  -v, --version             Output version information and exit.\n");
}

int main(int argc, char** argv) {
  long read_size_kb = 4;
  long stride_kb = 512;
  int max_aio = 128;

  struct option long_options[] = {
      {"read-size-kb", required_argument, 0, 'r'},
      {"stride-kb", required_argument, 0, 's'},
      {"queue-depth", required_argument, 0, 'q'},
      {"full-disk", no_argument, 0, 'f'},
      {"syslog", no_argument, 0, 'l'},
      {"silent", no_argument, &silent_mode, 1},
      {"help", no_argument, 0, 'h'},
      {"version", no_argument, 0, 'v'},
      {0, 0, 0, 0}};

  int opt;
  while ((opt = getopt_long(argc, argv, "r:s:q:flhv", long_options, NULL)) != -1)
  {
      switch (opt) {
          case 0:
              break;
          case 'r': read_size_kb = atol(optarg); break;
          case 's': stride_kb = atol(optarg); break;
          case 'q': max_aio = atoi(optarg); break;
          case 'f':
              full_disk_mode = 1;
              break;
          case 'l': syslog_mode = 1; break;
          case 'h': help(); return 0;
          case 'v': printf("disk-warmer version %s\n", DISK_WARMER_VERSION); return 0;
          default: help(); return 1;
      }
  }

  if (optind + 2 > argc) {
      fprintf(stderr, "Error: Missing <directory> and/or <device> arguments.\n\n");
      help();
      return 1;
  }

  const char* dir = argv[optind];
  const char* dev = argv[optind + 1];

  if (syslog_mode) {
      openlog("disk-warmer", LOG_PID, LOG_USER);
      if (full_disk_mode)
      {
          syslog(LOG_INFO, "Starting two-phase warmup for directory '%s' on device '%s'", dir, dev);
      }
      else
      {
          syslog(LOG_INFO, "Starting directory warmup for '%s' on device '%s'", dir, dev);
      }
  }

  struct stat stat_buf;
  if (stat(dev, &stat_buf) != 0) {
      perror("stat device");
      if (syslog_mode) syslog(LOG_ERR, "Failed to stat device %s: %m", dev);
      return 1;
  }

  if (!S_ISBLK(stat_buf.st_mode)) {
      fprintf(stderr, "Warning: Device %s is not a block device. Continuing anyway.\n", dev);
      if (syslog_mode) syslog(LOG_WARNING, "Device %s is not a block device.", dev);
  }

  const long long read_size = read_size_kb * 1024;
  const long long stride = stride_kb * 1024;

  int dev_fd = open(dev, O_RDONLY);
  if (dev_fd == -1) {
    perror("open device");
    return 1;
  }

  // Get device size and initialize bitmap
  off_t disk_size = get_device_size(dev_fd);
  struct warmed_bitmap warmed_bm;
  bitmap_init(&warmed_bm, disk_size, stride);

  // Initialize timing variables
  struct timeval overall_start, phase1_start, phase1_end, phase2_start, phase2_end;
  gettimeofday(&overall_start, NULL);

  // Phase 1: Discover and warm directory files
  if (!silent_mode)
      printf("=== Phase 1: Discovering and warming directory files ===\n");
  gettimeofday(&phase1_start, NULL);

  struct extent_list el;
  extent_list_init(&el);
  traverse_dir(dir, &el);

  if (!silent_mode)
      printf("Found %zu extents in directory to warm.\n", el.size);
  if (el.size == 0) {
      if (!silent_mode)
          printf("No files found in directory. Proceeding to warm entire disk.\n");
      if (syslog_mode)
          syslog(LOG_INFO, "No files found in directory.");
      gettimeofday(&phase1_end, NULL);
      print_timing("Phase 1 (discovery only)", get_time_diff(phase1_start, phase1_end));
  }
  else
  {
      qsort(el.data, el.size, sizeof(struct extent), compare_extents);
      if (!silent_mode)
          printf("Directory extents sorted for sequential reading.\n");
      if (syslog_mode)
          syslog(LOG_INFO, "Found %zu extents in directory to warm.", el.size);

      if (warm_extents(dev_fd, &el, &warmed_bm, "Phase 1 - Directory files", read_size, stride, max_aio) < 0)
      {
          extent_list_free(&el);
          bitmap_free(&warmed_bm);
          close(dev_fd);
          return 1;
      }
      gettimeofday(&phase1_end, NULL);
      print_timing("Phase 1 (directory warming)", get_time_diff(phase1_start, phase1_end));
  }

  // Phase 2: Warm remaining disk (optional)
  if (full_disk_mode)
  {
      if (!silent_mode)
          printf("\n=== Phase 2: Warming remaining disk blocks ===\n");
      if (syslog_mode)
          syslog(LOG_INFO, "Starting phase 2: warming remaining disk blocks");
      gettimeofday(&phase2_start, NULL);

      if (warm_remaining_disk(dev_fd, &warmed_bm, read_size, stride, max_aio) < 0)
      {
          extent_list_free(&el);
          bitmap_free(&warmed_bm);
          close(dev_fd);
          return 1;
      }
      gettimeofday(&phase2_end, NULL);
      print_timing("Phase 2 (remaining disk warming)", get_time_diff(phase2_start, phase2_end));

      double total_time = get_time_diff(overall_start, phase2_end);
      if (!silent_mode)
          printf("\n=== Two-phase disk warming completed successfully ===\n");
      print_timing("Total warming time", total_time);
  }
  else
  {
      double total_time = get_time_diff(overall_start, phase1_end);
      if (!silent_mode)
          printf("\n=== Directory warming completed successfully ===\n");
      print_timing("Total warming time", total_time);
  }

  if (syslog_mode)
  {
      if (full_disk_mode)
      {
          syslog(LOG_INFO, "Two-phase disk warming completed successfully.");
      }
      else
      {
          syslog(LOG_INFO, "Directory warming completed successfully.");
      }
      closelog();
  }

  extent_list_free(&el);
  bitmap_free(&warmed_bm);
  close(dev_fd);
  return 0;
} 