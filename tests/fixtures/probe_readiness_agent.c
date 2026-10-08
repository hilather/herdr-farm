/* Real native child whose argv storage temporarily exposes a process-title race. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <errno.h>
#include <sys/stat.h>
int main(int argc, char **argv) {
    if (argc == 2 && !strcmp(argv[1], "--version")) {
        puts("codex-cli 0.154.0"); return 0;
    }
    /* After the arguments-change exec, remain stable for identity observation. */
    int no_daemon = argc == 2 && !strcmp(argv[1], "--no-daemon");
    if (argc > 1 && !no_daemon) for (;;) pause();
    FILE *mode = fopen("../mode", "r");
    int mutation = mode ? fgetc(mode) : '0';
    if (mode) fclose(mode);
    if (mutation == 'z') {
        fputs("app server did not become ready: Error: File exists (os error 17)\n", stderr);
        return 17;
    }
    if (mutation == 'h') {
        const char *home = getenv("HOME");
        char directory[4096], link[4096], target[128];
        snprintf(directory, sizeof directory, "%s/.codex/app-server-control", home);
        snprintf(link, sizeof link, "%s/app-server-control.sock", directory);
        snprintf(target, sizeof target, "/tmp/codex-daemon-%u/fixture", (unsigned)geteuid());
        if (!no_daemon) {
            mkdir(directory, 0700);
            if (symlink(target, link) != 0) {
                perror("daemon control link: File exists");
                return 17;
            }
        }
        /* The server opens stderr outside the sandbox; home may be read-only. */
        fprintf(stderr, "probe-fixture started uid=%u\n", (unsigned)geteuid());
        char write_probe[4096];
        snprintf(write_probe, sizeof write_probe, "%s/.probe-fixture-ro-check", home);
        FILE *probe = fopen(write_probe, "wx");
        if (probe) {
            fclose(probe);
            unlink(write_probe);
        }
        fprintf(stderr, "probe-fixture home=%s\n", probe ? "rw" : "ro");
        fflush(stderr);
        mutation = '0';
    }
    if (mutation == 'w') {
        /* Nothing inside the worker sandbox is guaranteed writable (on CI the
           probe home and working directory are read-only), so the re-exec
           carries "already wrapped" in the environment, which the probe's
           identity check (executable + argv) does not compare. */
        if (!getenv("PROBE_FIXTURE_WRAPPED")) {
            /* Match the released sandbox wrapper's public process identity,
               then restore the exact agent argv by exec in the same child. */
            execl("/bin/sh", "/bin/sh", "-c",
                ": probe-fixture-wrapper; sleep 2; PROBE_FIXTURE_WRAPPED=1 exec \"$1\" --no-daemon",
                "herdr-farm-worker-sandbox", argv[0], (char *)NULL);
            return 3;
        }
        mutation = '0';
    }
    /* Wait until the public readiness request proves initial identity passed. */
    while (access("readiness-started", F_OK) != 0) usleep(10000);
    if (mutation == 'a') {
        execl(argv[0], argv[0], "unexpected-argument", (char *)NULL);
        return 3;
    }
    if (mutation == 'x') {
        execl("/usr/bin/sleep", "sleep", "60", (char *)NULL);
        return 3;
    }
    if (mutation != '0') {
        /* Linux's proctitle path reads argv[0] when arg_end is non-NUL.
           Cover both an empty title and one without a NUL in the read page. */
        size_t length = strlen(argv[0]) + 1;
        char *start = argv[0];
        unsigned long low, high;
        size_t span = length;
        char line[512];
        FILE *maps = fopen("/proc/self/maps", "r");
        if (!maps) return 2;
        while (fgets(line, sizeof line, maps)) {
            if (sscanf(line, "%lx-%lx", &low, &high) == 2 &&
                (unsigned long)start >= low && (unsigned long)start < high) {
                span = high - (unsigned long)start;
                break;
            }
        }
        fclose(maps);
        char *saved = malloc(span);
        memcpy(saved, start, span);
        memset(start, 'x', span);
        if (mutation == 'e') start[0] = 0;
        start[length - 1] = 'x';
        sleep(2);
        memcpy(start, saved, span);
        free(saved);
    }
    for (;;) pause();
}
