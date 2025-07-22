#define _POSIX_C_SOURCE 200809L
#ifdef __linux__
#include <libaio.h>
#ifdef HAVE_LIBURING
#include <liburing.h>
#endif
#endif
#include "page_cache_warmer.h"
#include <signal.h>
#include <stdlib.h>
#include <unistd.h>
#include <fcntl.h>
#include <stdio.h>

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

// libaio implementation
#if defined(__linux__) || defined(HAVE_LIBURING)
int io_warm_files(struct file_list *files, long long read_size, int queue_depth, const char* phase_name, const struct config* cfg) {
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
    int fds[queue_depth];
    off_t file_offsets[queue_depth];

    for(int i = 0; i < queue_depth; ++i) fds[i] = -1;

    size_t file_index = 0;
    int inflight = 0;

    while (file_index < files->count || inflight > 0) {
        int submitted = 0;
        while (inflight < queue_depth && file_index < files->count) {
            int q_idx = -1;
            for(int i = 0; i < queue_depth; ++i) {
                if(fds[i] == -1) {
                    q_idx = i;
                    break;
                }
            }
            if (q_idx == -1) break; // No free slots

            fds[q_idx] = open(files->files[file_index].path, O_RDONLY);
            if (fds[q_idx] < 0) {
                DEBUG_LOG(cfg, "Failed to open file %s", files->files[file_index].path);
                file_index++;
                continue;
            }
            file_offsets[q_idx] = 0;

            io_prep_pread(&iocbs[q_idx], fds[q_idx], buffers[q_idx], read_size, file_offsets[q_idx]);
            iocbsp[inflight] = &iocbs[q_idx];
            submitted++;
            inflight++;
        }

        if (submitted > 0) {
             if(io_submit(ctx, submitted, iocbsp) < submitted) {
                perror("io_submit");
                break;
             }
        }
        
        progress_print(phase_name, file_index, files->count);

        if (inflight > 0) {
            int completed = io_getevents(ctx, 1, inflight, events, NULL);
            if (completed < 0) {
                perror("io_getevents");
                break;
            }

            for (int i = 0; i < completed; ++i) {
                struct iocb *cb = events[i].obj;
                int q_idx = cb - iocbs;

                long bytes_read = events[i].res;
                if (bytes_read > 0) {
                    file_offsets[q_idx] += bytes_read;
                    if (file_offsets[q_idx] < files->files[file_index].size) { // Should be file associated with this fd
                         io_prep_pread(&iocbs[q_idx], fds[q_idx], buffers[q_idx], read_size, file_offsets[q_idx]);
                         if (io_submit(ctx, 1, &cb) < 1) {
                            perror("io_submit single");
                            close(fds[q_idx]);
                            fds[q_idx] = -1;
                            inflight--;
                         }
                         continue;
                    }
                }
                
                close(fds[q_idx]);
                fds[q_idx] = -1;
                inflight--;
                file_index++; // Consider file done
            }
        }
    }
    
    progress_print(phase_name, files->count, files->count);
    io_destroy(ctx);
    free_aligned_buffers(buffers, queue_depth);
    return 0;
}
#else
// Provide a stub for non-Linux platforms
int io_warm_files(struct file_list *files, long long read_size, int queue_depth, const char* phase_name, const struct config* cfg) {
    fprintf(stderr, "Asynchronous I/O is not supported on this platform. Files will not be warmed.\n");
    return -1;
}
#endif

#ifdef HAVE_LIBURING
// io_uring implementation
struct request {
    int fd;
    off_t offset;
    size_t total_size;
    int q_idx;
};

int io_warm_files_uring(struct file_list *files, long long read_size, int queue_depth, const char* phase_name, const struct config* cfg) {
    struct io_uring ring;
    if (io_uring_queue_init(queue_depth, &ring, 0) != 0) {
        perror("io_uring_queue_init");
        return -1;
    }

    char *buffers[queue_depth];
    if (allocate_aligned_buffers(buffers, queue_depth, read_size) < 0) {
        io_uring_queue_exit(&ring);
        return -1;
    }
    
    struct request requests[queue_depth];
    for(int i=0; i<queue_depth; ++i) requests[i].fd = -1;

    size_t file_index = 0;
    int inflight = 0;
    
    while(file_index < files->count || inflight > 0) {
        while(inflight < queue_depth && file_index < files->count) {
            int q_idx = -1;
            for(int i=0; i<queue_depth; ++i) {
                if(requests[i].fd == -1) {
                    q_idx = i;
                    break;
                }
            }
            if(q_idx == -1) break;

            int fd = open(files->files[file_index].path, O_RDONLY);
            if (fd < 0) {
                DEBUG_LOG(cfg, "Failed to open file %s", files->files[file_index].path);
                file_index++;
                continue;
            }

            requests[q_idx] = (struct request) {
                .fd = fd,
                .offset = 0,
                .total_size = files->files[file_index].size,
                .q_idx = q_idx
            };

            struct io_uring_sqe *sqe = io_uring_get_sqe(&ring);
            io_uring_prep_read(sqe, fd, buffers[q_idx], read_size, 0);
            io_uring_sqe_set_data(sqe, &requests[q_idx]);
            inflight++;
            file_index++;
        }

        if(io_uring_submit(&ring) < 0) {
            perror("io_uring_submit");
            break;
        }
        
        progress_print(phase_name, file_index - inflight, files->count);

        struct io_uring_cqe *cqe;
        int wait_res = io_uring_wait_cqe(&ring, &cqe);
        if (wait_res < 0) {
            perror("io_uring_wait_cqe");
            continue;
        }

        struct request *req = (struct request *)cqe->user_data;
        if(cqe->res > 0) {
            req->offset += cqe->res;
            if (req->offset < req->total_size) {
                struct io_uring_sqe *sqe = io_uring_get_sqe(&ring);
                io_uring_prep_read(sqe, req->fd, buffers[req->q_idx], read_size, req->offset);
                io_uring_sqe_set_data(sqe, req);
            } else {
                close(req->fd);
                req->fd = -1;
                inflight--;
            }
        } else {
            close(req->fd);
            req->fd = -1;
            inflight--;
        }
        io_uring_cqe_seen(&ring, cqe);
    }
    
    progress_print(phase_name, files->count, files->count);
    io_uring_queue_exit(&ring);
    free_aligned_buffers(buffers, queue_depth);
    return 0;
}
#endif 