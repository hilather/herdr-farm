#define _GNU_SOURCE
#include <dlfcn.h>
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdatomic.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <time.h>
static atomic_int elapsed_read;
/* Deterministic scheduler delay: advance this CLI's monotonic clock instead
   of sleeping or loading the host. Wall clock and other processes are untouched. */
int clock_gettime(clockid_t clock, struct timespec *value) {
    int (*real_clock)(clockid_t, struct timespec *) = dlsym(RTLD_NEXT, "clock_gettime");
    int result = real_clock(clock, value);
    if (!result && clock == CLOCK_MONOTONIC && atomic_load(&elapsed_read)) value->tv_sec += 11;
    return result;
}
/* Fault only the lab's selected regular artifact. No sockets or services. */
ssize_t read(int fd, void *buffer, size_t size) {
    ssize_t (*real_read)(int, void *, size_t) = dlsym(RTLD_NEXT, "read");
    static atomic_int injected;
    char descriptor[64], path[4096];
    snprintf(descriptor, sizeof(descriptor), "/proc/self/fd/%d", fd);
    ssize_t length = readlink(descriptor, path, sizeof(path)-1);
    const char *selected = getenv("CONTROL_COPY_SOURCE");
    if (length >= 0) {
        path[length] = 0;
        if (selected && !strcmp(path, selected) && !atomic_exchange(&injected, 1)) {
            const char *marker = getenv("CONTROL_COPY_INJECTED");
            if (marker) { int out = open(marker, O_WRONLY|O_CREAT|O_EXCL, 0600); if (out >= 0) close(out); }
            atomic_store(&elapsed_read, 1); /* Outlast the old ten-second read cap. */
            errno = EINTR;
            return -1;
        }
    }
    return real_read(fd, buffer, size);
}
