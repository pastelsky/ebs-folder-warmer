#include "disk_warmer.h"

// Common I/O buffer management
static int allocate_aligned_buffers(char **buffers, int count, size_t buffer_size) {
    for (int i = 0; i < count; i++) {
        if (posix_memalign((void **)&buffers[i], DEFAULT_ALIGNMENT_BYTES, buffer_size) != 0) {
            perror("posix_memalign for buffer");
            for (int j = 0; j < i; j++)
                free(buffers[j]);
            return -1;
        }
    }
    return 0;
}

static void free_aligned_buffers(char **buffers, int count) {
    for (int i = 0; i < count; i++) {
        free(buffers[i]);
    }
}

// Calculate total strides for progress tracking
static unsigned long long calculate_total_strides(const struct extent_list *list, long long stride) {
    unsigned long long total_strides = 0;
    for (size_t i = 0; i < list->count; i++) {
        total_strides += (list->extents[i].length + stride - 1) / stride;
    }
    return total_strides;
}

// libaio implementation
int io_warm_extents(int device_fd, const struct extent_list *list, 
                   struct warmed_bitmap *bitmap, const char *phase_name,
                   long long read_size, long long stride, int queue_depth, int debug_mode) {
    if (debug_mode) {
        fprintf(stderr, "[DEBUG] io_warm_extents: Starting libaio warming with %zu extents, queue_depth=%d\n", 
                list->count, queue_depth);
    }
    
    io_context_t ctx = {0};
    if (io_setup(queue_depth, &ctx) < 0) {
        perror("io_setup");
        return -1;
    }
    
    if (debug_mode) {
        fprintf(stderr, "[DEBUG] io_warm_extents: libaio context initialized successfully\n");
    }

    char *buffers[queue_depth];
    if (allocate_aligned_buffers(buffers, queue_depth, read_size) < 0) {
        if (debug_mode) {
            fprintf(stderr, "[DEBUG] io_warm_extents: Failed to allocate aligned buffers\n");
        }
        io_destroy(ctx);
        return -1;
    }
    if (debug_mode) {
        fprintf(stderr, "[DEBUG] io_warm_extents: Allocated %d aligned buffers of %lld bytes each\n", 
                queue_depth, read_size);
    }

    struct iocb iocbs[queue_depth];
    struct iocb *iocbsp[queue_depth];
    struct io_event events[queue_depth];

    size_t extent_index = 0;
    off_t extent_offset = 0;
    unsigned long long total_reads = 0;
    unsigned long long total_strides = calculate_total_strides(list, stride);

    while (extent_index < list->count) {
        int batch_size = 0;
        
        // Prepare batch of I/O operations
        while (batch_size < queue_depth && extent_index < list->count) {
            const struct extent *extent = &list->extents[extent_index];
            off_t remaining = extent->length - extent_offset;
            
            if (remaining <= 0) {
                extent_index++;
                extent_offset = 0;
                continue;
            }
            
            off_t read_offset = extent->physical_offset + extent_offset;
            io_prep_pread(&iocbs[batch_size], device_fd, buffers[batch_size], 
                         read_size, read_offset);
            iocbsp[batch_size] = &iocbs[batch_size];
            batch_size++;
            total_reads++;

            // Mark this range as warmed
            if (bitmap) {
                bitmap_mark_range(bitmap, read_offset, read_size);
            }

            extent_offset += stride;
            if (extent_offset >= extent->length) {
                extent_index++;
                extent_offset = 0;
            }
        }
        
        if (batch_size == 0) break;
        
        progress_print(phase_name, total_reads, total_strides);

        // Submit I/O operations
        int submitted = io_submit(ctx, batch_size, iocbsp);
        if (submitted != batch_size) {
            if (submitted < 0) {
                perror("io_submit");
            } else {
                fprintf(stderr, "io_submit: submitted only %d of %d\n", submitted, batch_size);
            }
            break;
        }
        
        // Wait for completions
        int completed = io_getevents(ctx, batch_size, batch_size, events, NULL);
        if (completed != batch_size) {
            if (completed < 0) {
                perror("io_getevents");
            } else {
                fprintf(stderr, "io_getevents: received only %d of %d\n", completed, batch_size);
            }
            break;
        }
    }

    progress_print(phase_name, total_strides, total_strides); // Final update to 100%
    printf("\n");
    syslog(LOG_INFO, "%s completed %llu reads.", phase_name, total_reads);
    
    if (debug_mode) {
        fprintf(stderr, "[DEBUG] io_warm_extents: Completed %llu reads across %llu strides\n", 
                total_reads, total_strides);
    }

    io_destroy(ctx);
    free_aligned_buffers(buffers, queue_depth);
    if (debug_mode) {
        fprintf(stderr, "[DEBUG] io_warm_extents: Cleanup completed, libaio context destroyed\n");
    }
    return 0;
}

int io_warm_remaining_disk(int device_fd, struct warmed_bitmap *bitmap,
                          long long read_size, long long stride, int queue_depth, int debug_mode) {
    off_t disk_size = device_get_size(device_fd);
    if (disk_size <= 0) {
        fprintf(stderr, "Could not determine device size\n");
        return -1;
    }

    io_context_t ctx = {0};
    if (io_setup(queue_depth, &ctx) < 0) {
        perror("io_setup");
        return -1;
    }

    char *buffers[queue_depth];
    if (allocate_aligned_buffers(buffers, queue_depth, read_size) < 0) {
        io_destroy(ctx);
        return -1;
    }

    struct iocb iocbs[queue_depth];
    struct iocb *iocbsp[queue_depth];
    struct io_event events[queue_depth];

    unsigned long long total_reads = 0;
    unsigned long long total_strides = (disk_size + stride - 1) / stride;
    off_t current_offset = 0;

    while (current_offset < disk_size) {
        int batch_size = 0;
        
        // Prepare batch of I/O operations
        while (batch_size < queue_depth && current_offset < disk_size) {
            // Skip if this block was already warmed
            if (bitmap_is_marked(bitmap, current_offset)) {
                current_offset += stride;
                continue;
            }

            off_t read_length = read_size;
            if (current_offset + read_length > disk_size) {
                read_length = disk_size - current_offset;
            }

            io_prep_pread(&iocbs[batch_size], device_fd, buffers[batch_size], 
                         read_length, current_offset);
            iocbsp[batch_size] = &iocbs[batch_size];
            batch_size++;
            total_reads++;
            current_offset += stride;
        }

        if (batch_size == 0) break;
        
        progress_print("Phase 2 - Remaining disk", total_reads, total_strides);

        // Submit and wait for I/O operations
        int submitted = io_submit(ctx, batch_size, iocbsp);
        if (submitted != batch_size) {
            if (submitted < 0) {
                perror("io_submit");
            } else {
                fprintf(stderr, "io_submit: submitted only %d of %d\n", submitted, batch_size);
            }
            break;
        }
        
        int completed = io_getevents(ctx, batch_size, batch_size, events, NULL);
        if (completed != batch_size) {
            if (completed < 0) {
                perror("io_getevents");
            } else {
                fprintf(stderr, "io_getevents: received only %d of %d\n", completed, batch_size);
            }
            break;
        }
    }

    progress_print("Phase 2 - Remaining disk", total_strides, total_strides);
    printf("\n");
    syslog(LOG_INFO, "Phase 2 completed %llu reads.", total_reads);

    io_destroy(ctx);
    free_aligned_buffers(buffers, queue_depth);
    return 0;
}

#ifdef HAVE_LIBURING
// io_uring implementation
int io_warm_extents_uring(int device_fd, const struct extent_list *list,
                         struct warmed_bitmap *bitmap, const char *phase_name,
                         long long read_size, long long stride, int queue_depth, int debug_mode) {
    if (debug_mode) {
        fprintf(stderr, "[DEBUG] io_warm_extents_uring: Starting io_uring warming with %zu extents, queue_depth=%d\n", 
                list->count, queue_depth);
    }
    
    struct io_uring ring;

    // Initialize io_uring with SQPOLL for better performance
    if (io_uring_queue_init(queue_depth, &ring, IORING_SETUP_SQPOLL) != 0) {
        // Fallback to regular mode if SQPOLL fails
        if (io_uring_queue_init(queue_depth, &ring, 0) != 0) {
            printf("io_uring initialization failed, falling back to libaio\n");
            return io_warm_extents(device_fd, list, bitmap, phase_name, 
                                 read_size, stride, queue_depth);
        }
    }

    char *buffers[queue_depth];
    if (allocate_aligned_buffers(buffers, queue_depth, read_size) < 0) {
        io_uring_queue_exit(&ring);
        return -1;
    }

    size_t extent_index = 0;
    off_t extent_offset = 0;
    unsigned long long total_reads = 0;
    unsigned long long total_strides = calculate_total_strides(list, stride);

    while (extent_index < list->count) {
        int batch_size = 0;
        
        // Prepare batch of I/O operations
        while (batch_size < queue_depth && extent_index < list->count) {
            const struct extent *extent = &list->extents[extent_index];
            off_t remaining = extent->length - extent_offset;
            
            if (remaining <= 0) {
                extent_index++;
                extent_offset = 0;
                continue;
            }
            
            off_t read_offset = extent->physical_offset + extent_offset;

            // Get submission queue entry
            struct io_uring_sqe *sqe = io_uring_get_sqe(&ring);
            if (!sqe) {
                break; // Ring is full, submit what we have
            }

            // Prepare read operation
            io_uring_prep_read(sqe, device_fd, buffers[batch_size], read_size, read_offset);
            io_uring_sqe_set_data(sqe, (void *)(uintptr_t)batch_size);

            batch_size++;
            total_reads++;

            // Mark this range as warmed
            if (bitmap) {
                bitmap_mark_range(bitmap, read_offset, read_size);
            }

            extent_offset += stride;
            if (extent_offset >= extent->length) {
                extent_index++;
                extent_offset = 0;
            }
        }

        if (batch_size == 0) break;

        progress_print(phase_name, total_reads, total_strides);

        // Submit operations
        int submitted = io_uring_submit(&ring);
        if (submitted != batch_size) {
            fprintf(stderr, "io_uring_submit: submitted only %d of %d\n", submitted, batch_size);
        }

        // Wait for completions
        for (int i = 0; i < submitted; i++) {
            struct io_uring_cqe *cqe;
            if (io_uring_wait_cqe(&ring, &cqe) != 0) {
                perror("io_uring_wait_cqe");
                break;
            }

            if (cqe->res < 0) {
                fprintf(stderr, "io_uring read error: %s\n", strerror(-cqe->res));
            }

            io_uring_cqe_seen(&ring, cqe);
        }
    }

    progress_print(phase_name, total_strides, total_strides);
    printf("\n");
    syslog(LOG_INFO, "%s completed %llu reads.", phase_name, total_reads);

    io_uring_queue_exit(&ring);
    free_aligned_buffers(buffers, queue_depth);
    return 0;
}

int io_warm_remaining_disk_uring(int device_fd, struct warmed_bitmap *bitmap,
                                long long read_size, long long stride, int queue_depth, int debug_mode) {
    off_t disk_size = device_get_size(device_fd);
    if (disk_size <= 0) {
        fprintf(stderr, "Could not determine device size\n");
        return -1;
    }

    struct io_uring ring;
    if (io_uring_queue_init(queue_depth, &ring, IORING_SETUP_SQPOLL) != 0) {
        if (io_uring_queue_init(queue_depth, &ring, 0) != 0) {
            printf("io_uring initialization failed, falling back to libaio\n");
            return io_warm_remaining_disk(device_fd, bitmap, read_size, stride, queue_depth);
        }
    }

    char *buffers[queue_depth];
    if (allocate_aligned_buffers(buffers, queue_depth, read_size) < 0) {
        io_uring_queue_exit(&ring);
        return -1;
    }

    unsigned long long total_reads = 0;
    unsigned long long total_strides = (disk_size + stride - 1) / stride;
    off_t current_offset = 0;

    while (current_offset < disk_size) {
        int batch_size = 0;
        
        // Prepare batch of I/O operations
        while (batch_size < queue_depth && current_offset < disk_size) {
            // Skip if this block was already warmed
            if (bitmap_is_marked(bitmap, current_offset)) {
                current_offset += stride;
                continue;
            }

            off_t read_length = read_size;
            if (current_offset + read_length > disk_size) {
                read_length = disk_size - current_offset;
            }

            struct io_uring_sqe *sqe = io_uring_get_sqe(&ring);
            if (!sqe) {
                break;
            }

            io_uring_prep_read(sqe, device_fd, buffers[batch_size], read_length, current_offset);
            io_uring_sqe_set_data(sqe, (void *)(uintptr_t)batch_size);

            batch_size++;
            total_reads++;
            current_offset += stride;
        }

        if (batch_size == 0) break;
        
        progress_print("Phase 2 - Remaining disk", total_reads, total_strides);

        int submitted = io_uring_submit(&ring);
        if (submitted != batch_size) {
            fprintf(stderr, "io_uring_submit: submitted only %d of %d\n", submitted, batch_size);
        }

        for (int i = 0; i < submitted; i++) {
            struct io_uring_cqe *cqe;
            if (io_uring_wait_cqe(&ring, &cqe) != 0) {
                perror("io_uring_wait_cqe");
                break;
            }

            if (cqe->res < 0) {
                fprintf(stderr, "io_uring read error: %s\n", strerror(-cqe->res));
            }

            io_uring_cqe_seen(&ring, cqe);
        }
    }

    progress_print("Phase 2 - Remaining disk", total_strides, total_strides);
    printf("\n");
    syslog(LOG_INFO, "Phase 2 completed %llu reads.", total_reads);

    io_uring_queue_exit(&ring);
    free_aligned_buffers(buffers, queue_depth);
    return 0;
}
#endif 