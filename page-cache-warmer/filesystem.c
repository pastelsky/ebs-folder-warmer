#include "page_cache_warmer.h"
#include <pthread.h>
#include <unistd.h>
#include <sys/stat.h>
#include <dirent.h>
#include <string.h>
#include <errno.h>
#include <stdio.h>
#include <stdlib.h>

struct path_queue_item {
    char path[MAX_PATH_LENGTH];
    int depth;
    struct path_queue_item *next;
};

struct worker_args {
    struct file_list *list;
    pthread_mutex_t *list_mutex;
    pthread_mutex_t *queue_mutex;
    pthread_cond_t *queue_cond;
    struct path_queue_item **head;
    int *queue_size;
    int *done;
    int max_depth;
};

static void enqueue_path(struct path_queue_item **head, const char *path, int depth) {
    struct path_queue_item *newItem = malloc(sizeof(struct path_queue_item));
    strncpy(newItem->path, path, MAX_PATH_LENGTH -1);
    newItem->path[MAX_PATH_LENGTH -1] = '\0';
    newItem->depth = depth;
    newItem->next = *head;
    *head = newItem;
}

static void *worker_thread(void *arg) {
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

        struct path_queue_item *current = *wargs->head;
        *wargs->head = current->next;
        (*wargs->queue_size)--;
        pthread_mutex_unlock(wargs->queue_mutex);

        DIR *dir = opendir(current->path);
        if (!dir) {
            free(current);
            continue;
        }

        struct dirent *entry;
        while ((entry = readdir(dir)) != NULL) {
            if (strcmp(entry->d_name, ".") == 0 || strcmp(entry->d_name, "..") == 0)
                continue;

            char full_path[MAX_PATH_LENGTH];
            snprintf(full_path, sizeof(full_path), "%s/%s", current->path, entry->d_name);

            struct stat st;
            if (lstat(full_path, &st) == 0) {
                if (S_ISDIR(st.st_mode)) {
                    if (wargs->max_depth == -1 || current->depth < wargs->max_depth) {
                        pthread_mutex_lock(wargs->queue_mutex);
                        enqueue_path(wargs->head, full_path, current->depth + 1);
                        (*wargs->queue_size)++;
                        pthread_cond_signal(wargs->queue_cond);
                        pthread_mutex_unlock(wargs->queue_mutex);
                    }
                } else if (S_ISREG(st.st_mode)) {
                    pthread_mutex_lock(wargs->list_mutex);
                    file_list_append(wargs->list, full_path, st.st_size);
                    pthread_mutex_unlock(wargs->list_mutex);
                }
            }
        }
        closedir(dir);
        free(current);
    }
    return NULL;
}

void discover_files(const char *directory_path, struct file_list *list, int current_depth, int max_depth, int num_threads) {
    pthread_t threads[num_threads];
    struct worker_args wargs;
    pthread_mutex_t list_mutex, queue_mutex;
    pthread_cond_t queue_cond;
    
    pthread_mutex_init(&list_mutex, NULL);
    pthread_mutex_init(&queue_mutex, NULL);
    pthread_cond_init(&queue_cond, NULL);

    struct path_queue_item *head = NULL;
    enqueue_path(&head, directory_path, current_depth);
    int queue_size = 1;
    int done = 0;

    wargs.list = list;
    wargs.list_mutex = &list_mutex;
    wargs.queue_mutex = &queue_mutex;
    wargs.queue_cond = &queue_cond;
    wargs.head = &head;
    wargs.queue_size = &queue_size;
    wargs.done = &done;
    wargs.max_depth = max_depth;

    for (int i = 0; i < num_threads; i++) {
        pthread_create(&threads[i], NULL, worker_thread, &wargs);
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
        usleep(10000); // Check every 10ms
    }

    for (int i = 0; i < num_threads; i++) {
        pthread_join(threads[i], NULL);
    }

    pthread_mutex_destroy(&list_mutex);
    pthread_mutex_destroy(&queue_mutex);
    pthread_cond_destroy(&queue_cond);
}

void file_list_init(struct file_list *list) {
    list->files = NULL;
    list->count = 0;
    list->capacity = 0;
}

void file_list_append(struct file_list *list, const char *path, off_t size) {
    if (list->count >= list->capacity) {
        size_t new_capacity = list->capacity == 0 ? 1024 : list->capacity * 2;
        struct file_info *new_files = realloc(list->files, new_capacity * sizeof(struct file_info));
        if (!new_files) {
            perror("realloc file_list");
            return;
        }
        list->files = new_files;
        list->capacity = new_capacity;
    }
    list->files[list->count].path = strdup(path);
    list->files[list->count].size = size;
    list->count++;
}

void file_list_free(struct file_list *list) {
    for (size_t i = 0; i < list->count; i++) {
        free(list->files[i].path);
    }
    free(list->files);
    list->files = NULL;
    list->count = 0;
    list->capacity = 0;
} 