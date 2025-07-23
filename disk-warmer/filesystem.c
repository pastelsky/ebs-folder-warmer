#include "disk_warmer.h"
#include <pthread.h>
#include <unistd.h>  // for usleep
#include <sys/stat.h> // For stat
#include <sys/types.h> // For __u64
#include <sys/ioctl.h> // For ioctl
#include <fcntl.h> // For open
#include <dirent.h> // For opendir, readdir, closedir
#include <string.h> // For strncpy, strcmp, snprintf
#include <errno.h> // For perror
#include <stdio.h> // For fprintf, perror
#include <stdlib.h> // For malloc, free
#ifdef __linux__
#include <linux/fiemap.h>
#include <linux/fs.h>
#include <linux/types.h>
#endif

// Structs and functions here
struct path_queue {
    char path[MAX_PATH_LENGTH];
    int depth;
    struct path_queue *next;
};

struct worker_args {
    struct extent_list *list;
    pthread_mutex_t *list_mutex;
    pthread_mutex_t *queue_mutex;
    pthread_cond_t *queue_cond;
    struct path_queue **head;
    struct path_queue **tail;
    int *queue_size;
    int *done;
    int max_depth;
};

static void *worker(void *arg) {
    struct worker_args *wargs = (struct worker_args *)arg;
    while (1) {
        pthread_mutex_lock(wargs->queue_mutex);
        while (*wargs->queue_size == 0 && !*wargs->done) {
            pthread_cond_wait(wargs->queue_cond, wargs->queue_mutex);
        }
        if (*wargs->queue_size == 0 && *wargs->done) {
            pthread_mutex_unlock(wargs->queue_mutex);
            break;
        }
        struct path_queue *current = *wargs->head;
        *wargs->head = current->next;
        if (!*wargs->head) *wargs->tail = NULL;
        (*wargs->queue_size)--;
        pthread_mutex_unlock(wargs->queue_mutex);
        if (wargs->max_depth >= 0 && current->depth > wargs->max_depth) {
            free(current);
            continue;
        }
        DIR *dir = opendir(current->path);
        if (!dir) {
            free(current);
            continue;
        }
        struct dirent *entry;
        while ((entry = readdir(dir))) {
            if (strcmp(entry->d_name, ".") == 0 || strcmp(entry->d_name, "..") == 0) continue;
            char path[MAX_PATH_LENGTH];
            snprintf(path, sizeof(path), "%s/%s", current->path, entry->d_name);
            struct stat st;
            if (lstat(path, &st) < 0) continue;
            if (S_ISDIR(st.st_mode)) {
                struct path_queue *new_q = malloc(sizeof(struct path_queue));
                if (!new_q) continue;
                strncpy(new_q->path, path, MAX_PATH_LENGTH);
                new_q->depth = current->depth + 1;
                new_q->next = NULL;
                pthread_mutex_lock(wargs->queue_mutex);
                if (*wargs->tail) (*wargs->tail)->next = new_q;
                else *wargs->head = new_q;
                *wargs->tail = new_q;
                (*wargs->queue_size)++;
                pthread_cond_broadcast(wargs->queue_cond);
                pthread_mutex_unlock(wargs->queue_mutex);
            } else if (S_ISREG(st.st_mode)) {
                pthread_mutex_lock(wargs->list_mutex);
                filesystem_extract_file_extents(path, wargs->list);
                pthread_mutex_unlock(wargs->list_mutex);
            } else if (S_ISLNK(st.st_mode)) {
                char target_path[MAX_PATH_LENGTH];
                ssize_t len = readlink(path, target_path, sizeof(target_path) - 1);
                if (len == -1) continue;
                target_path[len] = '\0';
                char final_path[MAX_PATH_LENGTH];
                if (target_path[0] == '/') {
                    strncpy(final_path, target_path, sizeof(final_path));
                } else {
                    snprintf(final_path, sizeof(final_path), "%s/%s", current->path, target_path);
                }
                if (stat(final_path, &st) == 0 && S_ISREG(st.st_mode)) {
                    pthread_mutex_lock(wargs->list_mutex);
                    filesystem_extract_file_extents(final_path, wargs->list);
                    pthread_mutex_unlock(wargs->list_mutex);
                }
            }
        }
        closedir(dir);
        free(current);
    }
    return NULL;
}

void filesystem_extract_file_extents(const char *file_path, struct extent_list *list) {
#ifdef __linux__
    int fd = open(file_path, O_RDONLY);
    if (fd == -1) {
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
    size_t fiemap_size = sizeof(struct fiemap) + (FIEMAP_EXTENT_BATCH_SIZE * sizeof(struct fiemap_extent));
    struct fiemap *fiemap = calloc(1, fiemap_size);
    if (!fiemap) {
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
        if (ioctl(fd, FS_IOC_FIEMAP, fiemap) == -1) {
            perror("FIEMAP");
            break;
        }
        if (fiemap->fm_mapped_extents == 0) break;
        for (unsigned i = 0; i < fiemap->fm_mapped_extents; i++) {
            struct fiemap_extent *ext = &fiemap->fm_extents[i];
            if (ext->fe_flags & FIEMAP_EXTENT_UNKNOWN) continue;
            extent_list_append(list, ext->fe_physical, ext->fe_length);
            offset = ext->fe_logical + ext->fe_length;
            if (ext->fe_flags & FIEMAP_EXTENT_LAST) offset = 0;
        }
    } while (offset > 0);
    free(fiemap);
    close(fd);
#else
    // Non-Linux stub: perhaps just append dummy extent or error
    fprintf(stderr, "FIEMAP not supported on this platform\n");
#endif
}

void filesystem_discover_extents(const char *directory_path, struct extent_list *list, int current_depth, int max_depth, int num_threads) {
    pthread_mutex_t list_mutex;
    pthread_mutex_init(&list_mutex, NULL);
    struct path_queue *queue_head = NULL, *queue_tail = NULL;
    struct path_queue *initial = malloc(sizeof(struct path_queue));
    if (!initial) return;
    strncpy(initial->path, directory_path, MAX_PATH_LENGTH);
    initial->depth = current_depth;
    initial->next = NULL;
    queue_head = queue_tail = initial;
    pthread_mutex_t queue_mutex;
    pthread_mutex_init(&queue_mutex, NULL);
    pthread_cond_t queue_cond;
    pthread_cond_init(&queue_cond, NULL);
    int queue_size = 1;
    int done = 0;
    struct worker_args wargs = {
        .list = list,
        .list_mutex = &list_mutex,
        .queue_mutex = &queue_mutex,
        .queue_cond = &queue_cond,
        .head = &queue_head,
        .tail = &queue_tail,
        .queue_size = &queue_size,
        .done = &done,
        .max_depth = max_depth
    };
    if (num_threads == 1) {
        worker(&wargs);
    } else {
        pthread_t threads[num_threads];
        for (int i = 0; i < num_threads; i++) {
            pthread_create(&threads[i], NULL, worker, &wargs);
        }
        while (1) {
            pthread_mutex_lock(&queue_mutex);
            if (queue_size == 0) {
                done = 1;
                pthread_cond_broadcast(&queue_cond);
                pthread_mutex_unlock(&queue_mutex);
                break;
            }
            pthread_mutex_unlock(&queue_mutex);
            usleep(10000);
        }
        for (int i = 0; i < num_threads; i++) {
            pthread_join(threads[i], NULL);
        }
    }
    pthread_mutex_destroy(&list_mutex);
    pthread_mutex_destroy(&queue_mutex);
    pthread_cond_destroy(&queue_cond);
} 